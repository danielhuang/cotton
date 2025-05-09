use async_compression::tokio::bufread::GzipDecoder;
use color_eyre::{
    eyre::{eyre, Result},
    Report,
};
use compact_str::{CompactString, ToCompactString};
use futures::{StreamExt, TryStreamExt};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::{
    fs::Permissions,
    io::{self, ErrorKind},
    os::unix::prelude::PermissionsExt,
    path::{Path, PathBuf},
    sync::Arc,
};
use std::{
    fs::{create_dir_all, exists, metadata, read_dir, remove_dir_all, set_permissions, File},
    os::unix::fs::symlink,
};
use tap::Pipe;
use tokio::{sync::Semaphore, task::JoinHandle};
use tokio_tar::Archive;
use tokio_util::io::StreamReader;

use crate::{
    cache::Cache,
    config::{client_auth, read_config},
    npm::{Dependency, DependencyTree},
    package::PackageMetadata,
    progress::{log_progress, log_verbose},
    scoped_path::scoped_join,
    util::{retry, VersionSpecifier, CLIENT, CLIENT_LIMIT},
};

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq, Clone)]
pub struct Plan {
    #[serde(flatten)]
    pub trees: FxHashMap<CompactString, DependencyTree>,
}

impl Plan {
    pub fn new(trees: FxHashMap<CompactString, DependencyTree>) -> Self {
        Self { trees }
    }

    pub fn satisfies(&self, package: &PackageMetadata) -> bool {
        let map: FxHashMap<_, _> = self
            .trees
            .values()
            .map(|x| (x.root.name.to_compact_string(), x.root.version.clone()))
            .collect();
        package.iter_all().all(|req| {
            if let Some(version) = map.get(&req.name) {
                if let VersionSpecifier::Range(range) = req.version {
                    return range.satisfies(version);
                }
            }
            false
        })
    }
}

pub fn tree_size(trees: &FxHashMap<CompactString, DependencyTree>) -> usize {
    trees.len()
        + trees
            .values()
            .map(|x| tree_size(&x.children))
            .sum::<usize>()
}

#[tracing::instrument]
async fn download_package(dep: &Dependency) -> Result<()> {
    let target_path = scoped_join(".cotton/store", dep.id())?;

    create_dir_all(&target_path)?;

    if metadata(target_path.join("_complete")).is_ok() {
        log_verbose(&format!("Skipped downloading {}", dep.id()));
        return Ok(());
    }

    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(CLIENT_LIMIT));
    let permit = S.acquire().await.unwrap();

    log_verbose(&format!("Downloading {}@{}", dep.name, dep.version));

    let registry_auth = read_config()
        .await?
        .registry
        .into_iter()
        .find(|x| dep.dist.tarball.starts_with(&x.url))
        .and_then(|x| x.auth);

    let mut res = CLIENT
        .get(&*dep.dist.tarball)
        .pipe(|x| client_auth(x, registry_auth.as_ref()))?
        .send()
        .await?
        .error_for_status()?
        .bytes_stream()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e));

    let res = {
        let (tx, rx) = flume::unbounded();
        tokio::spawn(async move {
            while let Some(buf) = res.next().await {
                if tx.send_async(buf).await.is_err() {
                    break;
                }
            }
            drop(permit);
        });
        rx.into_stream()
    };

    let reader = StreamReader::new(res);
    let reader = GzipDecoder::new(reader);

    let mut archive = Archive::new(reader);

    archive
        .unpack(&target_path)
        .await
        .map_err(|e| eyre!("{e:?}"))?;

    File::create(target_path.join("_complete"))?;

    log_progress(&format!("Downloaded {}", dep.id().bright_blue()));

    Ok(())
}

pub async fn download_package_shared(dep: Dependency) -> Result<()> {
    static CACHE: Lazy<Cache<Dependency, Result<(), Arc<Report>>>> = Lazy::new(|| {
        Cache::new(|key: Dependency| async move {
            retry(|| download_package(&key)).await.map_err(Arc::new)
        })
    });

    CACHE.get(dep).await.map_err(Report::msg)
}

fn hardlink_dir(src: PathBuf, dst: PathBuf) -> Result<()> {
    std::fs::create_dir_all(&dst)?;
    let dir = std::fs::read_dir(src)?;
    for entry in dir {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_dir() {
            hardlink_dir(entry.path(), dst.join(entry.file_name()))?;
        } else {
            std::fs::hard_link(entry.path(), dst.join(entry.file_name()))?;
        }
    }
    Ok(())
}

fn get_package_src(src: &Path) -> Result<PathBuf> {
    let mut dir = read_dir(src)?;
    while let Some(entry) = dir.next().transpose()? {
        let ty = entry.file_type()?;
        if ty.is_dir() {
            return Ok(entry.path());
        }
    }
    Err(Report::msg("No package src found"))
}

#[tracing::instrument]
pub async fn install_package(prefix: &[CompactString], dep: &Dependency) -> Result<()> {
    download_package_shared(dep.clone()).await?;

    let mut target_path = PathBuf::new();

    for segment in prefix {
        target_path.push(segment.as_str());
        target_path.push("node_modules");
    }

    target_path.push(&*dep.name);

    log_verbose(&format!("Installing {}", target_path.to_string_lossy()));

    target_path = scoped_join("node_modules", target_path)?;

    let install_marker = target_path.join(format!(".installed!{}", dep.id()));
    if exists(&install_marker)? {
        log_verbose(&format!(
            "Skipping installation for {}",
            dep.id().bright_blue()
        ));
        return Ok(());
    }

    let _ = remove_dir_all(&target_path);

    let src_path = scoped_join(".cotton/store", dep.id())?;

    hardlink_dir(get_package_src(&src_path)?, target_path)?;

    File::create(&install_marker)?;

    log_progress(&format!("Installed {}", dep.id().bright_blue()));

    Ok(())
}

fn warmup_dep_tree(dep: &DependencyTree) {
    tokio::spawn(download_package_shared(dep.root.clone()));
    for child in dep.children.values() {
        warmup_dep_tree(child);
    }
}

pub async fn execute_plan(plan: Plan) -> Result<()> {
    let (send, recv) = flume::unbounded();

    fn queue_install(
        send: flume::Sender<JoinHandle<Result<()>>>,
        tree: DependencyTree,
        prefix: Vec<CompactString>,
    ) -> Result<()> {
        send.clone().send(tokio::spawn(async move {
            install_package(&prefix, &tree.root).await?;

            for (_, dep) in tree.children {
                let mut prefix = prefix.clone();
                prefix.push(tree.root.name.clone());
                queue_install(send.clone(), dep, prefix)?;
            }

            Result::Ok(())
        }))?;

        Ok(())
    }

    for (_, tree) in plan.trees.into_iter() {
        warmup_dep_tree(&tree);
        queue_install(send.clone(), tree, vec![])?;
    }

    drop(send);

    while let Ok(x) = recv.recv_async().await {
        x.await??;
    }

    Ok(())
}

pub async fn setup_bins(plan: &Plan) -> Result<()> {
    for tree in plan.trees.values() {
        let dep = &tree.root;
        for (cmd, path) in &dep.bins {
            let path = path.to_compact_string();
            let mut path = PathBuf::from("../").join(&*dep.name).join(&*path);
            if !exists(PathBuf::from("node_modules/.bin").join(&path))? {
                path.set_extension("js");
            }
            if !cmd.contains('/') {
                let bin_path = PathBuf::from("node_modules/.bin").join(&**cmd);
                if let Err(e) = symlink(&path, &bin_path) {
                    if e.kind() != ErrorKind::AlreadyExists {
                        return Err(e.into());
                    }
                }
                set_permissions(&bin_path, Permissions::from_mode(0o755))?;
            }
        }
    }

    Ok(())
}
