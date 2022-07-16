use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use async_recursion::async_recursion;
use cached::proc_macro::cached;
use color_eyre::{
    eyre::{eyre, ContextCompat, Result},
    Report,
};
use compact_str::{CompactString, ToCompactString};
use futures::future::try_join_all;
use indexmap::IndexMap;
use itertools::Itertools;
use node_semver::Version;
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use tokio::sync::Semaphore;

use crate::{
    cache::Cache,
    package::{DepReq, Dist, Package},
    plan::download_package_shared,
    progress::{log_progress, log_verbose},
    util::{decode_json, get_node_cpu, get_node_os, retry, VersionReq, CLIENT_Z},
};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RegistryResponse {
    #[serde(rename = "dist-tags")]
    pub dist_tags: FxHashMap<CompactString, CompactString>,
    pub versions: IndexMap<Version, Package>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct PlatformMap(BTreeSet<CompactString>);

impl PlatformMap {
    pub fn allowed(&self) -> impl Iterator<Item = &str> {
        self.0
            .iter()
            .filter(|x| !x.starts_with('!'))
            .map(|x| x.as_str())
    }

    pub fn blocked(&self) -> impl Iterator<Item = &str> {
        self.0.iter().filter_map(|x| x.strip_prefix('!'))
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn is_supported(&self, platform: &str) -> bool {
        if self.is_empty() {
            true
        } else {
            self.allowed().any(|o| o == platform) && !self.blocked().any(|o| o == platform)
        }
    }
}

#[tracing::instrument]
pub async fn fetch_package(name: &str) -> Result<RegistryResponse> {
    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(128));
    let _permit = S.acquire().await.unwrap();

    retry(|| async {
        decode_json(
            &CLIENT_Z
                .get(format!("https://registry.npmjs.org/{}", name))
                .send()
                .await?
                .bytes()
                .await?,
        )
        .map_err(|e| eyre!("[{name}] {e}"))
    })
    .await
}

pub async fn fetch_package_cached(name: &str) -> Result<Arc<RegistryResponse>> {
    static CACHE: Lazy<Cache<CompactString, Result<Arc<RegistryResponse>, CompactString>>> =
        Lazy::new(|| {
            Cache::new(|key: CompactString, _| async move {
                fetch_package(&key)
                    .await
                    .map(Arc::new)
                    .map_err(|e| e.to_compact_string())
            })
        });

    CACHE
        .get(name.to_compact_string(), ())
        .await
        .map_err(Report::msg)
}

#[tracing::instrument]
#[cached(result)]
async fn fetch_dep_single(d: DepReq) -> Result<(Version, Package)> {
    let res = fetch_package_cached(&d.name).await?;

    if let VersionReq::Other(tag) = &d.version {
        let tag = res
            .dist_tags
            .get(tag)
            .wrap_err_with(|| eyre!("Version cannot be satisfied: {} {}", d.name, d.version))?;
        let version: Version = tag.parse()?;
        let package = res.versions.get(&version).wrap_err_with(|| {
            eyre!(
                "Tag refers to a version that does not exist: {} - {} refers to {}",
                d.name,
                d.version,
                version
            )
        })?;

        Ok((version, package.clone()))
    } else {
        let (version, package) = res
            .versions
            .iter()
            .sorted_by_key(|(v, _)| !v.is_prerelease())
            .rfind(|(v, _)| d.version.satisfies(v))
            .wrap_err_with(|| {
                eyre!(
                    "Version cannot be satisfied: expected {} {}",
                    d.name,
                    d.version
                )
            })?;

        Ok((version.clone(), package.clone()))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Hash, Clone, PartialOrd, Ord, Debug)]
pub struct DependencyTree {
    #[serde(flatten)]
    pub root: Dependency,
    pub children: BTreeMap<CompactString, Arc<DependencyTree>>,
}

impl DependencyTree {
    pub fn filter(&self, exclude: &FxHashSet<Dependency>) -> Self {
        Self {
            root: self.root.clone(),
            children: self
                .children
                .clone()
                .into_iter()
                .filter_map(|(name, tree)| {
                    if !exclude.contains(&tree.root) {
                        Some((name, Arc::new(tree.filter(exclude))))
                    } else {
                        None
                    }
                })
                .collect(),
        }
    }
}

#[derive(PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub name: CompactString,
    pub version: Version,
    pub dist: Dist,
    pub bins: BTreeMap<CompactString, CompactString>,
}

impl Dependency {
    pub fn id(&self) -> String {
        format!("{}@{}", self.name, self.version)
    }
}

#[async_recursion]
pub async fn fetch_dep(
    d: &DepReq,
    stack: &[(DepReq, Version)],
) -> Result<Option<Arc<DependencyTree>>> {
    let (version, package) = fetch_dep_single(d.clone()).await?;

    if !package.os.is_supported(get_node_os()) || !package.cpu.is_supported(get_node_cpu()) {
        if d.optional {
            return Ok(None);
        } else {
            return Err(Report::msg("Required dependency is not supported"));
        }
    }

    log_progress(&format!("Fetched {}", d.name.bright_blue()));

    let deps = try_join_all(package.iter().map(|d2| {
        let version = version.clone();
        async move {
            fetch_dep_cached(
                d2,
                stack
                    .iter()
                    .cloned()
                    .chain([(d.clone(), version)])
                    .collect_vec(),
            )
            .await
        }
    }))
    .await?
    .into_iter()
    .flatten();

    let tree = DependencyTree {
        children: deps
            .into_iter()
            .map(|x| (x.root.name.to_compact_string(), x))
            .collect(),
        root: Dependency {
            name: d.name.to_compact_string(),
            version: version.to_owned(),
            dist: package.dist.clone(),
            bins: package.bins(),
        },
    };

    tokio::spawn(download_package_shared(tree.root.clone()));

    Ok(Some(Arc::new(tree)))
}

type DepStack = Vec<(DepReq, Version)>;

pub async fn fetch_dep_cached(d: DepReq, stack: DepStack) -> Result<Option<Arc<DependencyTree>>> {
    type Args = (DepReq, DepStack);
    type Output = Option<Arc<DependencyTree>>;

    static CACHE: Lazy<Cache<Args, Result<Output, CompactString>, ()>> = Lazy::new(|| {
        Cache::new(|(d, stack): Args, _| async move {
            tokio::spawn(async move { fetch_dep(&d, &stack).await })
                .await
                .map_err(|e| e.to_compact_string())
                .and_then(|r| r.map_err(|e| e.to_compact_string()))
        })
    });

    if stack
        .iter()
        .any(|(d2, v)| d.name == d2.name && d.version.satisfies(v))
    {
        log_verbose(&format!(
            "Detected cyclic dependencies: {} > {} {}",
            stack
                .iter()
                .map(|(r, v)| format!("{}@{}", r.name, v).bright_blue().to_string())
                .join(" > "),
            d.name,
            d.version
        ));

        return Ok(None);
    }

    CACHE.get((d, stack), ()).await.map_err(Report::msg)
}

fn flatten_dep_tree(dep: &DependencyTree, map: &mut FxHashMap<Dependency, DependencyTree>) {
    if map.insert(dep.root.clone(), dep.clone()).is_none() {
        for dep in dep.children.values() {
            flatten_dep_tree(dep, map)
        }
    }
}

pub fn flatten_dep_trees<'a>(
    deps: impl Iterator<Item = &'a DependencyTree>,
) -> FxHashMap<Dependency, DependencyTree> {
    let mut set = Default::default();
    for dep in deps {
        flatten_dep_tree(dep, &mut set);
    }
    set
}
