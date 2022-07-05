mod cache;
mod npm;
mod package;
mod plan;
mod progress;
mod util;
mod watch;

use clap::Parser;
use color_eyre::eyre::{ContextCompat, Result};
use color_eyre::owo_colors::OwoColorize;
use compact_str::{CompactString, ToCompactString};
use futures::future::try_join_all;
use futures_lite::future::race;
use itertools::Itertools;
use npm::fetch_package;
use once_cell::sync::Lazy;
use package::{read_package, read_package_as_value, save_package, write_json, Package};
use plan::{flatten, tree_size};
use serde_json::Value;
use std::{env, path::PathBuf, process::exit, time::Instant};
use tikv_jemallocator::Jemalloc;
use tokio::fs::create_dir_all;
use tokio::{fs::read_to_string, process::Command};
use watch::async_watch;

use crate::{
    npm::fetch_dep,
    plan::{execute_plan, Plan},
    progress::PROGRESS_BAR,
};

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
    Add { name: CompactString },
    /// Run a script defined in package.json
    Run {
        name: CompactString,
        #[clap(long)]
        watch: Vec<PathBuf>,
    },
}

fn install_tracing() {
    use tracing_error::ErrorLayer;
    use tracing_subscriber::prelude::*;
    use tracing_subscriber::{fmt, EnvFilter};

    let fmt_layer = fmt::layer().with_target(false);
    let filter_layer = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(ErrorLayer::default())
        .init();
}

async fn prepare_plan(package: &Package) -> Result<Plan> {
    let deps = try_join_all(
        package
            .iter_with_dev()
            .map(|d| async move { fetch_dep(&d, &[]).await }),
    )
    .await?
    .into_iter()
    .flatten()
    .collect_vec();

    PROGRESS_BAR.set_message(format!("fetched {} root deps", deps.len().yellow()));

    let mut plan = Plan::new(
        deps.iter()
            .map(|x| (x.root.name.to_compact_string(), (**x).clone()))
            .collect(),
    );

    flatten(&mut plan.trees);

    PROGRESS_BAR.set_message(format!("planned {} deps", plan.trees.len().yellow()));

    Ok(plan)
}

async fn read_plan(path: &str) -> Result<Plan> {
    let plan = read_to_string(path).await?;
    Ok(serde_json::from_str(&plan)?)
}

pub async fn verify_installation(package: &Package) -> Result<bool> {
    let installed = read_plan("node_modules/.cotton/plan.json").await?;
    let lock_file = read_plan("cotton.lock").await?;

    if installed != lock_file {
        return Ok(false);
    }

    Ok(installed.satisfies(package))
}

async fn install() -> Result<(), color_eyre::Report> {
    let package = read_package().await?;

    if let Ok(true) = verify_installation(&package).await {
        return Ok(());
    }

    let start = Instant::now();

    init_storage().await?;

    let plan = {
        if let Ok(lock_file) = read_plan("cotton.lock").await {
            if lock_file.satisfies(&package) {
                lock_file
            } else {
                prepare_plan(&package).await?
            }
        } else {
            prepare_plan(&package).await?
        }
    };

    execute_plan(&plan).await?;

    write_json("node_modules/.cotton/plan.json", &plan).await?;
    write_json("cotton.lock", &plan).await?;

    PROGRESS_BAR.println(format!(
        "Installed {} packages in {}ms",
        tree_size(&plan.trees).yellow(),
        start.elapsed().as_millis().yellow()
    ));

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
    create_dir_all("node_modules/.cotton/tar").await?;
    create_dir_all("node_modules/.bin").await?;

    Ok(())
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);

#[tokio::main]
async fn main() -> Result<()> {
    install_tracing();
    color_eyre::install()?;

    match &ARGS.cmd {
        Subcommand::Install => {
            install().await?;
        }
        Subcommand::Update => {
            let package = read_package().await?;

            init_storage().await?;

            let start = Instant::now();

            let plan = prepare_plan(&package).await?;
            write_json("cotton.lock", &plan).await?;

            PROGRESS_BAR.println(format!(
                "Prepared {} packages in {}ms",
                tree_size(&plan.trees).yellow(),
                start.elapsed().as_millis().yellow()
            ));
        }
        Subcommand::Add { name } => {
            let mut package = read_package_as_value().await?;

            let res = fetch_package(name).await?;
            let latest = res
                .dist_tags
                .get("latest")
                .wrap_err("Package `latest` tag not specified")?;

            package
                .as_object_mut()
                .wrap_err("`package.json` is invalid")?
                .get_mut("dependencies")
                .wrap_err("`package.json` is missing `dependencies`")?
                .as_object_mut()
                .wrap_err("`package.json` contains invalid `dependencies`")?
                .insert(name.to_string(), Value::String(format!("^{}", latest)));

            save_package(&package).await?;

            PROGRESS_BAR.println(format!("Added {} {}", name.yellow(), latest.yellow()));

            install().await?;
        }
        Subcommand::Run { name, watch } => {
            join_paths()?;

            loop {
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

                        let exit_code = Command::new("sh")
                            .arg("-c")
                            .arg(script)
                            .kill_on_drop(true)
                            .spawn()?
                            .wait()
                            .await?
                            .code();

                        if let Some(exit_code) = exit_code {
                            exit(exit_code);
                        }

                        Ok(()) as Result<_>
                    },
                )
                .await?;
            }
        }
    }

    PROGRESS_BAR.finish_and_clear();

    exit(0);
}
