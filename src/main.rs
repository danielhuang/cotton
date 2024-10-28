mod cache;
mod config;
mod npm;
mod package;
mod plan;
mod progress;
mod resolve;
mod scoped_path;
mod util;
mod watch;

use async_recursion::async_recursion;
use clap::Parser;
use color_eyre::eyre::{eyre, ContextCompat, Result};
use color_eyre::owo_colors::OwoColorize;
use color_eyre::Help;
use compact_str::{CompactString, ToCompactString};
use config::read_config;
use futures::future::try_join_all;
use futures::lock::Mutex;
use futures_lite::future::race;
use itertools::Itertools;
use mimalloc::MiMalloc;
use multimap::MultiMap;
use nix::sys::signal::{self, Signal};
use nix::unistd::{execvp, Pid};
use node_semver::Version;
use npm::{fetch_package, Dependency};
use once_cell::sync::Lazy;
use package::{DepReq, Package};
use plan::tree_size;
use progress::{log_progress, log_verbose};
use rand::distributions::Alphanumeric;
use rand::Rng;
use resolve::{Graph, Lockfile};
use rustc_hash::FxHashSet;
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::env::{current_dir, current_exe, set_current_dir, set_var, temp_dir};
use std::ffi::{CString, OsStr, OsString};
use std::fs::remove_dir_all;
use std::io::ErrorKind;
use std::os::unix::fs::symlink;
use std::os::unix::prelude::OsStrExt;
use std::{env, path::PathBuf, process::exit, time::Instant};
use tokio::fs::{create_dir, create_dir_all, metadata};
use tokio::{fs::read_to_string, process::Command};
use tracing_error::ErrorLayer;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use util::{read_package, read_package_or_default, save_package, write_json};
use watch::async_watch;
use which::which;

use crate::npm::DependencyTree;
use crate::scoped_path::scoped_join;
use crate::util::load_graph_from_lockfile;
use crate::{
    plan::{execute_plan, Plan},
    progress::PROGRESS_BAR,
};

#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
pub struct Args {
    /// Print verbose logs (including progress indicators)
    #[clap(short, long, global = true)]
    verbose: bool,
    /// Prevent any modifications to the lockfile
    #[clap(long, global = true)]
    immutable: bool,
    /// Run in a custom working directory
    #[clap(long, global = true, alias = "cwd")]
    working_dir: Option<PathBuf>,
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
    Add {
        names: Vec<CompactString>,
        /// Add to `devDependencies` instead of `dependencies`
        #[clap(short = 'D', long)]
        dev: bool,
        /// Pin dependencies to a specific version
        #[clap(long, alias = "exact")]
        pin: bool,
    },
    /// Run a script defined in package.json
    Run {
        name: CompactString,
        #[clap(long)]
        watch: Vec<PathBuf>,
    },
    /// Clean packages installed in `node_modules` and remove cache
    Clean,
    /// Update packages specified in package.json to the latest available version
    Upgrade {
        /// Pin dependencies to a specific version
        #[clap(long)]
        pin: bool,
    },
    /// Execute a command that is not specified as a script
    Exec { exe: OsString, args: Vec<OsString> },
    /// Remove package from package.json
    Remove {
        names: Vec<CompactString>,
        /// Remove from `devDependencies` instead of `dependencies`
        #[clap(short = 'D', long)]
        dev: bool,
    },
    /// Find all uses of a given package
    Why {
        name: CompactString,
        version: Option<Version>,
    },
    /// Create new projects from a `create-` starter kit
    Create { name: CompactString },
    /// Download (if needed) and execute a command
    #[clap(name = "x")]
    DownloadAndExec { name: OsString, args: Vec<OsString> },
}

async fn prepare_plan(package: &Package) -> Result<Plan> {
    log_progress("Preparing");

    let mut graph = load_graph_from_lockfile().await;

    if !ARGS.immutable {
        graph.append(package.iter_all(), true).await?;
        write_json("cotton.lock", Lockfile::new(graph.clone())).await?;
    }

    log_progress("Retrieved dependency graph");

    let trees = graph.build_trees(&package.iter_all().collect_vec())?;
    log_progress(&format!("Fetched {} root deps", trees.len().yellow()));

    let plan = Plan::new(
        trees
            .iter()
            .map(|x| (x.root.name.to_compact_string(), x.clone()))
            .collect(),
    );

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

async fn exec_install_script(root: &Dependency, stack: &[CompactString]) -> Result<()> {
    let path = stack.join("/node_modules/");

    let dir = scoped_join("node_modules", path)?;

    for script_name in ["preinstall", "install", "postinstall"] {
        if let Some(script) = root.scripts.get(script_name) {
            PROGRESS_BAR.suspend(|| {
                println!("Executing {script_name} script for {}", stack.join(" > "));
            });

            let mut child = Command::new(shell().await?)
                .arg("-c")
                .arg(script)
                .current_dir(&dir)
                .env("PATH", new_path()?)
                .spawn()?;

            if !child.wait().await?.success() {
                return Err(eyre!("Install script unsuccessful"));
            }
        }
    }

    Ok(())
}

#[async_recursion]
async fn exec_install_scripts(tree: &DependencyTree, stack: &mut Vec<CompactString>) -> Result<()> {
    exec_install_script(&tree.root, stack).await?;

    stack.push(tree.root.name.clone());
    for tree in tree.children.values() {
        exec_install_scripts(tree, stack).await?;
    }
    stack.pop().unwrap();

    Ok(())
}

async fn install() -> Result<()> {
    let package = read_package().await?;

    init_storage().await?;
    let config = read_config().await?;

    let start = Instant::now();

    let plan = prepare_plan(&package).await?;
    let size = tree_size(&plan.trees);

    if matches!(verify_installation(&package, &plan).await, Ok(true)) {
        log_verbose("Packages already installed")
    } else {
        execute_plan(plan.clone()).await?;

        PROGRESS_BAR.suspend(|| {
            if size > 0 {
                println!(
                    "Installed {} packages in {}ms",
                    size.yellow(),
                    start.elapsed().as_millis().yellow()
                )
            }
        });

        if config.allow_install_scripts {
            for (name, tree) in plan.trees.iter() {
                exec_install_scripts(tree, &mut vec![name.clone()]).await?;
            }
        }

        write_json("node_modules/.cotton/plan.json", &plan).await?;
    }

    PROGRESS_BAR.finish_and_clear();

    Ok(())
}

fn new_path() -> Result<OsString> {
    let path = env::var_os("PATH").unwrap_or_default();
    let mut paths = env::split_paths(&path).collect::<Vec<_>>();
    let new = PathBuf::from("node_modules/.bin");
    paths.insert(0, new.canonicalize().unwrap_or(new));
    let new_path = env::join_paths(paths)?;
    Ok(new_path)
}

fn join_paths() -> Result<()> {
    let path = new_path()?;
    log_verbose(&format!("Setting PATH to {path:?}"));
    env::set_var("PATH", path);

    Ok(())
}

pub async fn init_storage() -> Result<()> {
    create_dir_all(".cotton/store").await?;
    create_dir_all("node_modules/.cotton").await?;
    create_dir_all("node_modules/.bin").await?;

    Ok(())
}

async fn add_packages(names: &[CompactString], dev: bool, pin: bool) -> Result<()> {
    let mut package: Value = read_package_or_default().await?;
    let dependencies = package
        .as_object_mut()
        .wrap_err("`package.json` is invalid")?
        .entry(if dev {
            "devDependencies"
        } else {
            "dependencies"
        })
        .or_insert(Value::Object(Default::default()))
        .as_object_mut()
        .wrap_err("`package.json` contains non-object dependencies field")?;

    log_progress("Resolving packages");

    for (name, res) in try_join_all(names.iter().map(|name| async move {
        PROGRESS_BAR.inc_length(1);
        let x = fetch_package(name).await.map(|res| (name, res));
        PROGRESS_BAR.inc(1);
        log_progress(&format!("Resolved {name}"));
        x
    }))
    .await?
    {
        let latest = res
            .dist_tags
            .get("latest")
            .wrap_err("Package `latest` tag not specified")?;

        let version = if pin {
            latest.to_string()
        } else {
            format!("^{latest}")
        };

        dependencies.insert(name.to_string(), Value::String(version.to_string()));

        PROGRESS_BAR.suspend(|| println!("Added {} {}", name.yellow(), version.yellow()));
    }

    save_package(&package).await?;

    Ok(())
}

pub async fn shell() -> Result<String> {
    for candidate in [
        "/bin/zsh",
        "/usr/bin/zsh",
        "/bin/bash",
        "/usr/bin/bash",
        "/bin/sh",
        "/usr/bin/sh",
    ] {
        if metadata(candidate).await.is_ok() {
            return Ok(candidate.to_string());
        }
    }
    Err(eyre!("No shell found"))
}

fn build_map(graph: &Graph) -> Result<MultiMap<(CompactString, Version), DepReq>> {
    let mut map = MultiMap::new();

    for (from, to) in graph.relations.iter() {
        for child_req in to.package.iter() {
            let child_dep = graph.resolve_req(&child_req)?;
            map.insert(
                (child_dep.package.name.clone(), child_dep.version),
                from.clone(),
            );
        }
    }

    Ok(map)
}

#[tracing::instrument]
fn exec_with_args(exe: &OsStr, args: &[OsString]) -> Result<()> {
    let exe = CString::new(exe.as_bytes().to_vec()).map_err(|_| eyre!("invalid path"))?;

    let mut args = args
        .iter()
        .map(|x| CString::new(x.as_bytes().to_vec()).map_err(|_| eyre!("invalid arguments")))
        .collect::<Result<Vec<_>>>()?;

    args.insert(0, exe.clone());
    execvp(&exe, &args)?;

    Ok(())
}

async fn install_bin_temp(package_name: &str) -> Result<()> {
    let orig_dir = current_dir()?;

    let dir_name: String = rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(10)
        .map(char::from)
        .collect();

    let mut temp_dir = temp_dir();
    temp_dir.push(dir_name);
    create_dir(&temp_dir).await?;
    set_current_dir(&temp_dir)?;
    log_verbose(&format!("Now in {temp_dir:?}"));

    save_package(&Value::Object(Map::new())).await?;
    add_packages(&[package_name.to_compact_string()], false, false).await?;
    install().await?;
    set_var(
        "npm_config_user_agent",
        "yarn/1.22.19 npm/none cotton/0.0.0",
    );
    symlink(current_exe()?, "node_modules/.bin/yarn")?;
    join_paths()?;

    set_current_dir(&orig_dir)?;
    log_verbose(&format!("Now in {orig_dir:?}"));

    Ok(())
}

pub static ARGS: Lazy<Args> = Lazy::new(Args::parse);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(ErrorLayer::default())
        .init();

    color_eyre::install()?;

    if let Some(cwd) = &ARGS.working_dir {
        set_current_dir(cwd)?;
    }

    match &ARGS.cmd {
        Subcommand::Install => {
            install().await?;
        }
        Subcommand::Update => {
            if ARGS.immutable {
                return Err(
                    eyre!("Cannot update lockfile").suggestion("Remove the --immutable flag")
                );
            }

            let package = read_package().await?;

            init_storage().await?;

            let start = Instant::now();

            let mut graph = Graph::default();
            graph.append(package.iter_all(), false).await?;
            write_json("cotton.lock", Lockfile::new(graph.clone())).await?;

            PROGRESS_BAR.suspend(|| {
                println!(
                    "Prepared {} packages in {}ms",
                    graph.relations.len().yellow(),
                    start.elapsed().as_millis().yellow()
                )
            });
        }
        Subcommand::Add { names, dev, pin } => {
            if names.is_empty() {
                PROGRESS_BAR.suspend(|| println!("Note: no packages specified"));
            }

            add_packages(names, *dev, *pin).await?;
        }
        Subcommand::Run { name, watch } => {
            join_paths()?;

            loop {
                let child_mutex = Mutex::new(None);

                race(
                    async {
                        let event = async_watch(watch.iter().map(|x| x.as_ref())).await?;
                        PROGRESS_BAR.suspend(|| {
                            println!(
                                "{} File modified: {}",
                                " WATCH ".on_purple(),
                                event.paths[0].to_string_lossy()
                            )
                        });
                        PROGRESS_BAR.finish_and_clear();

                        Ok(())
                    },
                    async {
                        let package = read_package().await?;

                        let script = package
                            .scripts
                            .get(name)
                            .wrap_err(format!("Script `{name}` is not defined"))?
                            .as_str()
                            .wrap_err(format!("Script `{name}` is not a string"))?;

                        install().await?;

                        let child = Command::new(shell().await?).arg("-c").arg(script).spawn()?;

                        let mut child_mutex = child_mutex.lock().await;
                        *child_mutex = Some(child);

                        let exit_code = child_mutex.as_mut().unwrap().wait().await?.code();

                        if let Some(exit_code) = exit_code {
                            exit(exit_code);
                        }

                        Ok(()) as Result<_>
                    },
                )
                .await?;

                let mut child = child_mutex.lock().await;
                if let Some(child) = child.as_mut() {
                    if let Some(pid) = child.id() {
                        signal::kill(Pid::from_raw(pid as _), Signal::SIGINT)?;
                        child.wait().await?;
                    }
                }
            }
        }
        Subcommand::Clean => {
            for dir in ["node_modules", ".cotton"] {
                match remove_dir_all(dir) {
                    Ok(()) => {}
                    Err(e) if e.kind() == ErrorKind::NotFound => {}
                    r => r?,
                }
            }
        }
        Subcommand::Upgrade { pin } => {
            let package = read_package().await?;
            add_packages(
                &package.dependencies.keys().cloned().collect_vec(),
                false,
                *pin,
            )
            .await?;
            add_packages(
                &package.dev_dependencies.keys().cloned().collect_vec(),
                true,
                *pin,
            )
            .await?;
        }
        Subcommand::Exec { exe, args } => {
            install().await?;
            join_paths()?;

            exec_with_args(exe, args)?;
        }
        Subcommand::Remove { names, dev } => {
            if names.is_empty() {
                PROGRESS_BAR.suspend(|| println!("Note: no packages specified"));
            }

            let mut package: Value = read_package_or_default().await?;
            let dependencies = package
                .as_object_mut()
                .wrap_err("`package.json` is invalid")?
                .entry(if *dev {
                    "devDependencies"
                } else {
                    "dependencies"
                })
                .or_insert(Value::Object(Default::default()))
                .as_object_mut()
                .wrap_err("`package.json` contains non-object dependencies field")?;

            for name in names {
                dependencies
                    .remove(&name.to_string())
                    .wrap_err(eyre!("Package `{name}` is not specified in `package.json`"))?;
            }

            log_progress(&format!("Removed {} dependencies", names.len()));

            save_package(&package).await?;
        }
        Subcommand::Why { name, version } => {
            let package = read_package().await?;

            let graph = load_graph_from_lockfile().await;

            let map = build_map(&graph)?;

            let mut seen = FxHashSet::default();
            let mut queue = VecDeque::new();

            if let Some(version) = version {
                queue.push_back((name.clone(), version.clone()));
            } else {
                for (req, resolved) in graph.relations.iter() {
                    if name == req.name {
                        queue.push_back((name.clone(), resolved.version.clone()));
                    }
                }
            }

            if queue.is_empty() {
                return Err(eyre!("Package {} is not used", name));
            }

            while let Some((name, version)) = queue.pop_front() {
                if seen.insert((name.clone(), version.clone())) {
                    if let Some(required_by) = map.get_vec(&(name.clone(), version.clone())) {
                        let required_by: FxHashSet<_> = required_by
                            .iter()
                            .map(|x| graph.resolve_req(x))
                            .try_collect()?;
                        if !required_by.is_empty() {
                            println!(
                                "{}",
                                format!("{}@{} is used by:", name.yellow(), version).bold()
                            );
                            for dep in required_by {
                                queue.push_back((dep.package.name.clone(), dep.version.clone()));
                                println!(" - {}@{}", dep.package.name, dep.version);
                            }
                            println!();
                        }
                    } else if package
                        .iter_all()
                        .any(|x| x.name == name && x.version.satisfies(&version))
                    {
                        println!(
                            "{}",
                            format!("{}@{} is used by package.json", name.yellow(), version).bold()
                        );
                        println!();
                    } else {
                        return Err(eyre!("Package {}@{} is not used", name, version));
                    }
                }
            }

            println!("Analyzed {} packages", seen.len().yellow());
        }
        Subcommand::Create { name } => {
            let name = format!("create-{name}");
            install_bin_temp(&name).await?;
            exec_with_args(OsStr::new(&name), &[])?;
        }
        Subcommand::DownloadAndExec { name, args } => {
            if let Err(e) = which(name) {
                log_verbose(&e.to_string());
                install_bin_temp(name.to_str().wrap_err("package name invalid")?).await?;
            }
            exec_with_args(name, args)?;
        }
    }

    PROGRESS_BAR.finish_and_clear();

    exit(0);
}
