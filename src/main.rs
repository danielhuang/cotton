mod cache;
mod npm;
mod package;
mod plan;
mod progress;
mod scoped_path;
mod util;
mod watch;

use clap::Parser;
use color_eyre::eyre::{ContextCompat, Result};
use color_eyre::owo_colors::OwoColorize;
use compact_str::{CompactString, ToCompactString};
use futures::lock::Mutex;
use futures_lite::future::race;
use nix::sys::signal::{self, Signal};
use nix::unistd::Pid;
use npm::{fetch_package, Graph, Lockfile};
use once_cell::sync::Lazy;
use package::Package;
use plan::{flatten, tree_size};
use progress::log_progress;
use serde_json::Value;
use std::{env, path::PathBuf, process::exit, time::Instant};
#[cfg(target_os = "linux")]
use tikv_jemallocator::Jemalloc;
use tokio::fs::create_dir_all;
use tokio::{fs::read_to_string, process::Command};
use tracing_error::ErrorLayer;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use util::{read_json, read_package, read_package_as_value, save_package, write_json};
use watch::async_watch;

use crate::{
    plan::{execute_plan, Plan},
    progress::PROGRESS_BAR,
};

#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    #[clap(short, long)]
    verbose: bool,
    #[clap(subcommand)]
    cmd: Subcommand,
}

#[derive(Parser, Debug, Clone)]
pub enum Subcommand {
    /// Install packages defined in package.json
    Install,
    /// Prepare and save a newly planned lockfile
    Update,
    /// Add package to package.json
    Add { names: Vec<CompactString> },
    /// Run a script defined in package.json
    Run {
        name: CompactString,
        #[clap(long)]
        watch: Vec<PathBuf>,
    },
}

async fn prepare_plan(package: &Package) -> Result<Plan> {
    log_progress("Preparing");

    let mut graph: Graph = read_json("cotton.lock").await.unwrap_or_default();
    graph.append(package.iter_with_dev()).await?;
    write_json("cotton.lock", Lockfile::new(graph.clone())).await?;

    log_progress("Retrieved dependency graph");

    let trees = graph.build_trees(package.iter_with_dev())?;
    log_progress(&format!("Fetched {} root deps", trees.len().yellow()));

    let mut plan = Plan::new(
        trees
            .iter()
            .map(|x| (x.root.name.to_compact_string(), (**x).clone()))
            .collect(),
    );

    flatten(&mut plan.trees);

    log_progress(&format!(
        "Planned {} dependencies",
        plan.trees.len().yellow()
    ));

    Ok(plan)
}

async fn read_plan(path: &str) -> Result<Plan> {
    let plan = read_to_string(path).await?;
    Ok(serde_json::from_str(&plan)?)
}

pub async fn verify_installation(package: &Package, plan: &Plan) -> Result<bool> {
    let installed = read_plan("node_modules/.cotton/plan.json").await?;

    if &installed != plan {
        return Ok(false);
    }

    Ok(installed.satisfies(package))
}

async fn install() -> Result<()> {
    let package = read_package().await?;

    init_storage().await?;

    let start = Instant::now();

    let plan = prepare_plan(&package).await?;

    if !matches!(verify_installation(&package, &plan).await, Ok(true)) {
        execute_plan(&plan).await?;
        write_json("node_modules/.cotton/plan.json", &plan).await?;

        PROGRESS_BAR.println(format!(
            "Installed {} packages in {}ms",
            tree_size(&plan.trees).yellow(),
            start.elapsed().as_millis().yellow()
        ));
    }

    PROGRESS_BAR.finish_and_clear();

    Ok(())
}

fn join_paths() -> Result<()> {
    if let Some(path) = env::var_os("PATH") {
        let mut paths = env::split_paths(&path).collect::<Vec<_>>();
        paths.push(PathBuf::from("node_modules/.bin"));
        let new_path = env::join_paths(paths)?;
        env::set_var("PATH", &new_path);
    }

    Ok(())
}

pub async fn init_storage() -> Result<()> {
    create_dir_all("node_modules/.cotton/store").await?;
    create_dir_all("node_modules/.bin").await?;

    Ok(())
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .init();

    color_eyre::install()?;

    match &ARGS.cmd {
        Subcommand::Install => {
            install().await?;
        }
        Subcommand::Update => {
            let package = read_package().await?;

            init_storage().await?;

            let start = Instant::now();

            let mut graph = Graph::default();
            graph.append(package.iter_with_dev()).await?;
            write_json("cotton.lock", Lockfile::new(graph.clone())).await?;

            PROGRESS_BAR.println(format!(
                "Prepared {} packages in {}ms",
                graph.relations.len().yellow(),
                start.elapsed().as_millis().yellow()
            ));
        }
        Subcommand::Add { names } => {
            if names.is_empty() {
                PROGRESS_BAR.println("Note: no packages specified");
            }

            let mut package = read_package_as_value().await?;

            let dependencies = package
                .as_object_mut()
                .wrap_err("`package.json` is invalid")?
                .entry("dependencies")
                .or_insert(Value::Object(Default::default()))
                .as_object_mut()
                .wrap_err("`package.json` contains invalid `dependencies`")?;

            for name in names {
                let res = fetch_package(name).await?;
                let latest = res
                    .dist_tags
                    .get("latest")
                    .wrap_err("Package `latest` tag not specified")?;

                dependencies.insert(name.to_string(), Value::String(format!("^{}", latest)));

                PROGRESS_BAR.println(format!("Added {} {}", name.yellow(), latest.yellow()));
            }

            save_package(&package).await?;

            install().await?;
        }
        Subcommand::Run { name, watch } => {
            join_paths()?;

            loop {
                let child_pid = Mutex::new(None);

                race(
                    async {
                        let event = async_watch(watch.iter().map(|x| x.as_ref())).await?;
                        PROGRESS_BAR.println(format!(
                            "{} File modified: {}",
                            " WATCH ".on_purple(),
                            event.paths[0].to_string_lossy()
                        ));
                        PROGRESS_BAR.finish_and_clear();

                        Ok(())
                    },
                    async {
                        let package = read_package().await?;

                        let script = package
                            .scripts
                            .get(name)
                            .wrap_err(format!("Script `{}` is not defined", name))?
                            .as_str()
                            .wrap_err(format!("Script `{}` is not a string", name))?;

                        install().await?;

                        let mut child = Command::new("sh").arg("-c").arg(script).spawn()?;

                        *child_pid.lock().await = child.id();

                        let exit_code = child.wait().await?.code();

                        if let Some(exit_code) = exit_code {
                            exit(exit_code);
                        }

                        Ok(()) as Result<_>
                    },
                )
                .await?;

                let pid = *child_pid.lock().await;
                if let Some(pid) = pid {
                    signal::kill(Pid::from_raw(pid as _), Signal::SIGINT)?;
                }
            }
        }
    }

    PROGRESS_BAR.finish_and_clear();

    exit(0);
}
