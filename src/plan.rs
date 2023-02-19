use async_compression::tokio::bufread::GzipDecoder;
use color_eyre::{
    eyre::{eyre, Result},
    Report,
};
use compact_str::{CompactString, ToCompactString};
use futures::TryStreamExt;
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use std::{
    fs::Permissions,
    io,
    os::unix::prelude::PermissionsExt,
    path::{Path, PathBuf},
};
use tokio::{
    fs::{
        create_dir_all, metadata, read_dir, remove_dir_all, remove_file, set_permissions, symlink,
        File,
    },
    sync::Semaphore,
    task::{spawn_blocking, JoinHandle},
};
use tokio_tar::Archive;
use tokio_util::io::StreamReader;

use crate::{
    cache::Cache,
    npm::{Dependency, DependencyTree},
    package::Package,
    progress::{log_progress, log_verbose, log_warning},
    scoped_path::scoped_join,
    util::{retry, VersionReq, CLIENT, CLIENT_LIMIT},
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

    pub fn satisfies(&self, package: &Package) -> bool {
        let map: FxHashMap<_, _> = self
            .trees
            .values()
            .map(|x| (x.root.name.to_compact_string(), x.root.version.clone()))
            .collect();
        package.iter_with_dev().all(|req| {
            if let Some(version) = map.get(&req.name) {
                if let VersionReq::Range(range) = req.version {
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
    let target_path = scoped_join("node_modules/.cotton/store", dep.id())?;

    create_dir_all(&target_path).await?;

    if metadata(&target_path.join("_complete")).await.is_ok() {
        log_verbose(&format!("Skipped downloading {}", dep.id()));
        return Ok(());
    }

    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(CLIENT_LIMIT));
    let permit = S.acquire().await.unwrap();

    log_verbose(&format!("Downloading {}@{}", dep.name, dep.version));

    let res = CLIENT
        .get(&*dep.dist.tarball)
        .send()
        .await?
        .error_for_status()?
        .bytes_stream()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e));

    drop(permit);

    let reader = StreamReader::new(res);
    let reader = GzipDecoder::new(reader);

    let mut archive = Archive::new(reader);

    archive
        .unpack(&target_path)
        .await
        .map_err(|e| eyre!("{e:?}"))?;

    File::create(&target_path.join("_complete")).await?;

    log_progress(&format!("Downloaded {}", dep.id().bright_blue()));

    Ok(())
}

pub async fn download_package_shared(dep: Dependency) -> Result<()> {
    static CACHE: Lazy<Cache<Dependency, Result<(), CompactString>>> = Lazy::new(|| {
        Cache::new(|key: Dependency, _| async move {
            retry(|| async { download_package(&key).await })
                .await
                .map_err(|e| e.to_compact_string())
        })
    });

    CACHE.get(dep, ()).await.map_err(Report::msg)
}

async fn hardlink_dir(src: PathBuf, dst: PathBuf) -> Result<()> {
    fn hardlink_dir_sync(src: PathBuf, dst: PathBuf) -> io::Result<()> {
        std::fs::create_dir_all(&dst)?;
        let dir = std::fs::read_dir(src)?;
        for entry in dir {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                hardlink_dir_sync(entry.path(), dst.join(entry.file_name()))?;
            } else {
                std::fs::hard_link(entry.path(), dst.join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    Ok(spawn_blocking(move || hardlink_dir_sync(src, dst)).await??)
}

async fn get_package_src(src: &Path) -> Result<PathBuf> {
    let mut dir = read_dir(src).await?;
    while let Some(entry) = dir.next_entry().await? {
        let ty = entry.file_type().await?;
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
    if metadata(&install_marker).await.is_ok() {
        log_verbose(&format!(
            "Skipping installation for {}",
            dep.id().bright_blue()
        ));
        return Ok(());
    }

    let _ = remove_dir_all(&target_path).await;

    let src_path = scoped_join("node_modules/.cotton/store", dep.id())?;

    hardlink_dir(get_package_src(&src_path).await?, target_path).await?;

    if prefix.is_empty() {
        for (cmd, path) in &dep.bins {
            let path = path.to_compact_string();
            let mut path = PathBuf::from("../").join(&*dep.name).join(&*path);
            if metadata(PathBuf::from("node_modules/.bin").join(&path))
                .await
                .is_err()
            {
                path.set_extension("js");
            }
            if !cmd.contains('/') {
                let bin_path = PathBuf::from("node_modules/.bin").join(&**cmd);
                let _ = remove_file(&bin_path).await;
                if symlink(&path, &bin_path).await.is_err() {
                    log_warning(&format!("Unable to save binary: {cmd}"));
                }
                if set_permissions(&bin_path, Permissions::from_mode(0o755))
                    .await
                    .is_err()
                {
                    log_warning(&format!("Unable to set permissions: {cmd}"));
                }
            }
        }
    }

    File::create(&install_marker).await?;

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
