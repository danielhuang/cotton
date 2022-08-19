use cached::proc_macro::cached;
use color_eyre::{
    eyre::{eyre, ContextCompat, Result},
    Help, Report,
};
use compact_str::{CompactString, ToCompactString};
use dashmap::DashMap;
use indexmap::IndexMap;
use itertools::Itertools;
use node_semver::Version;
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::fmt::Debug;
use std::{
    collections::{BTreeMap, BTreeSet},
    mem::take,
    sync::Arc,
};
use tokio::{
    sync::{
        mpsc::{unbounded_channel, UnboundedSender},
        Semaphore,
    },
    task::JoinHandle,
};

use crate::{
    cache::Cache,
    package::{DepReq, Dist, Package, Subpackage},
    plan::download_package_shared,
    progress::{log_progress, log_verbose},
    util::{decode_json, retry, VersionReq, CLIENT_LIMIT, CLIENT_Z},
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
    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(CLIENT_LIMIT));
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
async fn fetch_dep_single(d: DepReq) -> Result<(Version, Subpackage)> {
    let res = fetch_package_cached(&d.name).await?;

    log_progress(&format!("Fetched {}", d.name.bright_blue()));

    if let VersionReq::Other(tag) = &d.version {
        let tag = res
            .dist_tags
            .get(tag)
            .wrap_err_with(|| eyre!("Version cannot be satisfied: {} {}", d.name, d.version))?;
        let version = Version::parse(&tag)?;
        let package = res.versions.get(&version).wrap_err_with(|| {
            eyre!(
                "Tag refers to a version that does not exist: {} - {} refers to {}",
                d.name,
                d.version,
                version
            )
        })?;

        Ok((version, package.clone().sub()))
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

        Ok((version.clone(), package.clone().sub()))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct DependencyTree {
    #[serde(flatten)]
    pub root: Dependency,
    pub children: FxHashMap<CompactString, Arc<DependencyTree>>,
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

#[derive(PartialEq, Eq, Hash, Debug, Clone, Serialize, Deserialize)]
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

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Graph {
    #[serde(flatten)]
    pub relations: FxHashMap<DepReq, (Version, Subpackage)>,
}

impl Graph {
    pub async fn append(&mut self, remaining: impl Iterator<Item = DepReq>) -> Result<()> {
        fn queue_resolve(
            send: UnboundedSender<JoinHandle<Result<()>>>,
            req: DepReq,
            relations: Arc<DashMap<DepReq, Option<(Version, Subpackage)>>>,
        ) -> Result<()> {
            if relations.contains_key(&req) {
                return Ok(());
            }

            relations.insert(req.clone(), None);

            send.clone().send(tokio::spawn(async move {
                let (version, subpackage) = fetch_dep_single(req.clone()).await?;

                if subpackage.supported() {
                    tokio::spawn(download_package_shared(Dependency {
                        name: req.name.to_compact_string(),
                        version: version.clone(),
                        dist: subpackage.dist.clone(),
                        bins: subpackage.bins().into_iter().collect(),
                    }));
                }

                relations.insert(req, Some((version, subpackage.clone())));

                for child_req in subpackage.iter() {
                    queue_resolve(send.clone(), child_req, relations.clone())?;
                }

                Ok(()) as Result<_>
            }))?;

            Ok(())
        }

        let relations: Arc<DashMap<_, _>> = Arc::new(
            take(&mut self.relations)
                .into_iter()
                .map(|x| (x.0, Some(x.1)))
                .collect(),
        );

        let (send, mut recv) = unbounded_channel();

        for req in remaining {
            queue_resolve(send.clone(), req, relations.clone())?;
        }

        drop(send);

        while let Some(f) = recv.recv().await {
            f.await??;
        }

        self.relations = relations
            .iter()
            .map(|x| (x.key().clone(), x.value().clone().unwrap()))
            .collect();

        Ok(())
    }

    fn build_tree(
        &self,
        req: &DepReq,
        stack: &mut Vec<(DepReq, Version)>,
    ) -> Result<Option<Arc<DependencyTree>>> {
        if stack
            .iter()
            .any(|(d2, v)| req.name == d2.name && req.version.satisfies(v))
        {
            log_verbose(&format!(
                "Detected cyclic dependencies: {} > {} {}",
                stack
                    .iter()
                    .map(|(r, v)| format!("{}@{}", r.name, v).bright_blue().to_string())
                    .join(" > "),
                req.name,
                req.version
            ));

            return Ok(None);
        }

        let (version, package) = self
            .relations
            .get(req)
            .wrap_err("A transitive dependency is not found")
            .suggestion("Regenerate the lockfile")?
            .clone();

        if !package.supported() {
            if req.optional {
                return Ok(None);
            } else {
                return Err(Report::msg("Required dependency is not supported"));
            }
        }

        let mut deps = vec![];
        for dep in package.iter() {
            stack.push((req.clone(), version.clone()));
            if let Some(tree) = self.build_tree(&dep, stack)? {
                deps.push(tree);
            }
            stack.pop().unwrap();
        }

        let tree = DependencyTree {
            children: deps
                .into_iter()
                .map(|x| (x.root.name.to_compact_string(), x))
                .collect(),
            root: Dependency {
                name: req.name.to_compact_string(),
                version,
                dist: package.dist.clone(),
                bins: package.bins().into_iter().collect(),
            },
        };

        Ok(Some(Arc::new(tree)))
    }

    pub fn build_trees(
        &self,
        reqs: impl Iterator<Item = DepReq>,
    ) -> Result<Vec<Arc<DependencyTree>>> {
        let v: Result<Vec<_>> = reqs.map(|x| self.build_tree(&x, &mut vec![])).collect();
        let v = v?;
        let v = v.into_iter().flatten().collect();
        Ok(v)
    }
}

#[derive(Serialize, Default)]
pub struct Lockfile {
    #[serde(flatten)]
    pub relations: BTreeMap<DepReq, (Version, Subpackage)>,
}

impl Lockfile {
    pub fn new(graph: Graph) -> Self {
        Self {
            relations: graph.relations.into_iter().collect(),
        }
    }
}
