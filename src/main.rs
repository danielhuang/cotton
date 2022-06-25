mod npm;
mod package;
mod plan;
mod progress;
mod util;

use clap::Parser;
use color_eyre::eyre::{ContextCompat, Result};
use color_eyre::owo_colors::OwoColorize;
use futures::future::try_join_all;
use itertools::Itertools;
use std::{env, path::PathBuf, process::exit, time::Instant};

use npm::fetch_package;
use package::{read_package, read_package_as_value, save_package, Package};
use serde_json::Value;
use tokio::{
    fs::{read_to_string, File},
    io::AsyncWriteExt,
    process::Command,
};

use crate::{
    npm::fetch_dep,
    plan::{execute_plan, Plan},
    progress::PROGRESS_BAR,
};

/// Simple program to greet a person
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(subcommand)]
    cmd: Subcommand,
}

#[derive(Parser, Debug)]
enum Subcommand {
    /// Install packages defined in package.json
    Install,
    /// Add package to package.json
    Add { name: String },
    /// Run a script defined in package.json
    Run { name: String },
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

    let mut plan = Plan::new(deps.iter().map(|x| (**x).clone()).collect());
    plan.extract();
    plan.cleanup();

    PROGRESS_BAR.set_message(format!("planned {} deps", plan.deps.len().yellow()));

    Ok(plan)
}

pub async fn is_install_completed(package: &Package) -> bool {
    if let Ok(plan) = read_to_string("node_modules/.cotton/plan.json").await {
        if let Ok(plan) = serde_json::from_str::<Plan>(&plan) {
            if plan.satisfies(package) {
                return true;
            }
        }
    }
    false
}

async fn install() -> Result<(), color_eyre::Report> {
    let package = read_package().await?;

    if is_install_completed(&package).await {
        return Ok(());
    }

    let start = Instant::now();

    let plan = prepare_plan(&package).await?;

    execute_plan(&plan).await?;

    File::create("node_modules/.cotton/plan.json")
        .await?
        .write_all(serde_json::to_string(&plan)?.as_bytes())
        .await?;

    PROGRESS_BAR.println(format!(
        "Installed {} packages in {}ms",
        plan.flat_deps().len().yellow(),
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

#[tokio::main]
async fn main() -> Result<()> {
    install_tracing();
    color_eyre::install()?;

    let args = Args::parse();

    match args.cmd {
        Subcommand::Install => {
            install().await?;
        }
        Subcommand::Add { name } => {
            let mut package = read_package_as_value().await?;

            let res = fetch_package(&name).await?;
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
                .insert(name, Value::String(latest.to_string()));

            save_package(&package).await?;
        }
        Subcommand::Run { name } => {
            install().await?;
            let package = read_package().await?;

            let script = package
                .scripts
                .get(&name)
                .wrap_err(format!("Script `{}` is not defined", name))?;

            join_paths()?;

            let exit_code = Command::new("sh")
                .arg("-c")
                .arg(script)
                .spawn()?
                .wait()
                .await?
                .code();

            if let Some(exit_code) = exit_code {
                exit(exit_code);
            }
        }
    }

    PROGRESS_BAR.finish_and_clear();

    exit(0);
}
