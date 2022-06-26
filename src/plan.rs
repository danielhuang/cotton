use std::{collections::BTreeSet, path::PathBuf};

use async_compression::tokio::write::GzipDecoder;
use async_recursion::async_recursion;
use color_eyre::eyre::Result;
use futures::{future::try_join_all, StreamExt, TryStreamExt};
use itertools::Itertools;
use multimap::MultiMap;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use safe_path::scoped_join;
use serde::{Deserialize, Serialize};
use tokio::{
    fs::{create_dir_all, metadata, remove_dir_all, remove_file, rename, symlink, File},
    io::AsyncWriteExt,
};
use tokio_tar::Archive;

use crate::{
    npm::{flatten_deps, ExactDep},
    package::Package,
    progress::PROGRESS_BAR,
    util::{PartialRange, CLIENT},
};

#[derive(Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct Plan {
    pub deps: BTreeSet<ExactDep>,
}

impl Plan {
    pub fn new(deps: BTreeSet<ExactDep>) -> Self {
        Self { deps }
    }

    pub fn rootable(&self) -> Vec<ExactDep> {
        let flat = flatten_deps(self.deps.iter());
        let map: MultiMap<_, _> = flat.into_iter().map(|x| (x.name.to_string(), x)).collect();
        map.keys()
            .map(|x| {
                map.get_vec(x)
                    .unwrap()
                    .iter()
                    .max_by_key(|x| &x.version)
                    .unwrap()
            })
            .cloned()
            .collect()
    }

    pub fn extract(&mut self) {
        let rootable = self.rootable();
        for r in rootable {
            self.deps.insert(r.clone());
        }
    }

    pub fn cleanup(&mut self) {
        let rootable: FxHashSet<_> = self.rootable().into_iter().map(|x| x.as_single()).collect();
        let mut deps = self.deps.iter().cloned().collect_vec();
        for dep in deps.iter_mut() {
            dep.remove_deps(&rootable);
        }
        self.deps = deps.into_iter().collect();
    }

    pub fn flat_deps(&self) -> BTreeSet<ExactDep> {
        flatten_deps(self.deps.iter())
    }

    pub fn satisfies(&self, package: &Package) -> bool {
        let map: FxHashMap<_, _> = self
            .deps
            .iter()
            .map(|x| (x.name.to_string(), x.version.clone()))
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
pub async fn download_package(dep: &ExactDep) -> Result<()> {
    let target_path = scoped_join("node_modules/.cotton/tar", dep.tar())?;
    let target_part_path = scoped_join("node_modules/.cotton/tar", dep.tar_part())?;

    create_dir_all(target_part_path.parent().unwrap()).await?;

    if metadata(&target_path).await.is_ok() {
        return Ok(());
    }

    let mut res = CLIENT.get(&dep.dist.tarball).send().await?.bytes_stream();
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

#[tracing::instrument]
pub async fn extract_package(prefix: &[&str], dep: &ExactDep) -> Result<()> {
    let mut target_path = PathBuf::new();

    for segment in prefix {
        target_path.push(segment);
        target_path.push("node_modules");
    }

    target_path.push(&dep.name);

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
            let path = path.to_string();
            let mut path = PathBuf::from("../").join(&dep.name).join(path);
            if metadata(PathBuf::from("node_modules/.bin").join(&path))
                .await
                .is_err()
            {
                path.set_extension("js");
            }
            if !cmd.contains('/') {
                let _ = remove_file(PathBuf::from("node_modules/.bin").join(cmd)).await;
                symlink(&path, PathBuf::from("node_modules/.bin").join(cmd)).await?;
            }
        }
    }

    PROGRESS_BAR.set_message(format!("extracted {}", dep.id().bright_blue()));

    Ok(())
}

#[async_recursion]
#[tracing::instrument]
pub async fn extract_dep(prefix: &[&str], dep: &ExactDep) -> Result<()> {
    extract_package(prefix, dep).await?;

    try_join_all(dep.deps.iter().map(|inner_dep| async {
        let mut prefix = prefix.to_vec();
        prefix.push(&dep.name);
        extract_dep(&prefix, inner_dep).await?;

        Ok(()) as Result<_>
    }))
    .await?;

    Ok(())
}

pub async fn execute_plan(plan: &Plan) -> Result<()> {
    create_dir_all("node_modules/.cotton/tar").await?;
    create_dir_all("node_modules/.bin").await?;

    try_join_all(
        plan.flat_deps()
            .into_iter()
            .map(|x| async move { download_package(&x).await }),
    )
    .await?;

    try_join_all(
        plan.deps
            .iter()
            .map(|x| async move { extract_dep(&[], x).await }),
    )
    .await?;

    Ok(())
}
