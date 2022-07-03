use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

use async_compression::tokio::write::GzipDecoder;
use async_recursion::async_recursion;
use color_eyre::{eyre::Result, Report};
use compact_str::{CompactString, ToCompactString};
use futures::{future::try_join_all, StreamExt, TryStreamExt};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use safe_path::scoped_join;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, metadata, remove_dir_all, remove_file, rename, symlink, File},
    io::AsyncWriteExt,
    sync::Semaphore,
};
use tokio_tar::Archive;

use crate::{
    cache::Cache,
    npm::{flatten_dep_trees, Dependency, DependencyTree},
    package::Package,
    progress::PROGRESS_BAR,
    util::{PartialRange, CLIENT},
};

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Plan {
    pub trees: BTreeMap<CompactString, DependencyTree>,
}

impl Plan {
    pub fn new(trees: BTreeMap<CompactString, DependencyTree>) -> Self {
        Self { trees }
    }

    pub fn flatten(&mut self) {
        let mut flat_deps = self.flat_deps();
        let current_root_names: FxHashSet<_> = self
            .trees
            .values()
            .map(|x| x.root.name.to_compact_string())
            .collect();
        for dep in flat_deps.clone() {
            if current_root_names.contains(&dep.name) {
                flat_deps.remove(&dep);
            }
        }
        let mut hoisted: FxHashMap<_, Dependency> = FxHashMap::default();
        for dep in flat_deps {
            if let Some(prev) = hoisted.get(&dep.name) {
                if dep.version > prev.version {
                    hoisted.insert(dep.name.to_compact_string(), dep);
                }
            } else {
                hoisted.insert(dep.name.to_compact_string(), dep);
            }
        }
        for tree in self.trees.values_mut() {
            *tree = tree.filter(&hoisted.values().cloned().collect());
        }
        for item in hoisted.values() {
            self.trees.insert(
                item.name.to_compact_string(),
                DependencyTree {
                    root: item.clone(),
                    children: Default::default(),
                },
            );
        }
    }

    pub fn flat_deps(&self) -> BTreeSet<Dependency> {
        flatten_dep_trees(self.trees.values())
    }

    pub fn satisfies(&self, package: &Package) -> bool {
        let map: FxHashMap<_, _> = self
            .trees
            .values()
            .map(|x| (x.root.name.to_compact_string(), x.root.version.clone()))
            .collect();
        package.iter_with_dev().all(|req| {
            if let Some(version) = map.get(&req.name) {
                if let PartialRange::Range(range) = req.version {
                    return version.satisfies(&range);
                }
            }
            false
        })
    }
}

#[tracing::instrument]
async fn download_package(dep: &Dependency) -> Result<()> {
    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(48));
    let _permit = S.acquire().await.unwrap();

    let target_path = scoped_join("node_modules/.cotton/tar", dep.tar())?;
    let target_part_path = scoped_join("node_modules/.cotton/tar", dep.tar_part())?;

    create_dir_all(target_part_path.parent().unwrap()).await?;

    if metadata(&target_path).await.is_ok() {
        return Ok(());
    }

    let mut res = CLIENT.get(&*dep.dist.tarball).send().await?.bytes_stream();
    let target = File::create(&target_part_path).await?;

    let mut target = GzipDecoder::new(target);

    while let Some(bytes) = res.next().await {
        let mut bytes = bytes?;
        target.write_all_buf(&mut bytes).await?;
    }

    target.flush().await?;

    rename(&target_part_path, &target_path).await?;

    PROGRESS_BAR.set_message(format!("downloaded {}", dep.id().bright_blue()));

    Ok(())
}

pub async fn download_package_shared(dep: Dependency) -> Result<()> {
    static CACHE: Lazy<Cache<Dependency, Result<(), CompactString>>> = Lazy::new(|| {
        Cache::new(|key: Dependency| async move {
            download_package(&key)
                .await
                .map_err(|e| e.to_compact_string())
        })
    });

    CACHE.get(dep).await.map_err(Report::msg)
}

#[tracing::instrument]
pub async fn extract_package(prefix: &[&str], dep: &Dependency) -> Result<()> {
    let mut target_path = PathBuf::new();

    for segment in prefix {
        target_path.push(segment);
        target_path.push("node_modules");
    }

    target_path.push(&*dep.name);

    target_path = scoped_join("node_modules", target_path)?;

    let _ = remove_dir_all(&target_path).await;
    create_dir_all(&target_path).await?;

    let mut a =
        Archive::new(File::open(scoped_join("node_modules/.cotton/tar", dep.tar())?).await?);

    let mut entries = a.entries()?;

    while let Some(mut file) = entries.try_next().await? {
        let target_file = scoped_join(
            &target_path,
            file.path()?.components().skip(1).collect::<PathBuf>(),
        )?;
        create_dir_all(&target_file.parent().unwrap()).await?;
        if let Err(e) = file.unpack(&target_file).await {
            PROGRESS_BAR.println(format!(
                "{} ({}) {}",
                "Warning:".on_yellow(),
                dep.id().bright_blue(),
                e
            ));
        }
    }

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
                let _ = remove_file(PathBuf::from("node_modules/.bin").join(&**cmd)).await;
                symlink(&path, PathBuf::from("node_modules/.bin").join(&**cmd)).await?;
            }
        }
    }

    PROGRESS_BAR.set_message(format!("extracted {}", dep.id().bright_blue()));

    Ok(())
}

#[async_recursion]
#[tracing::instrument]
pub async fn extract_dep(prefix: &[&str], dep: &DependencyTree) -> Result<()> {
    extract_package(prefix, &dep.root).await?;

    try_join_all(dep.children.values().map(|inner_dep| async {
        let mut prefix = prefix.to_vec();
        prefix.push(&dep.root.name);
        extract_dep(&prefix, inner_dep).await?;

        Ok(()) as Result<_>
    }))
    .await?;

    Ok(())
}

pub async fn execute_plan(plan: &Plan) -> Result<()> {
    try_join_all(
        plan.flat_deps()
            .into_iter()
            .map(|x| async move { download_package_shared(x).await }),
    )
    .await?;

    try_join_all(
        plan.trees
            .values()
            .map(|x| async move { extract_dep(&[], x).await }),
    )
    .await?;

    Ok(())
}
