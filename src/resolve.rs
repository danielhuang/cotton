use crate::npm::{Dependency, DependencyTree};
use crate::package::{DepReq, Subpackage, VersionedSubpackage};
use crate::plan::download_package_shared;
use crate::progress::log_verbose;
use crate::{npm, ARGS};
use color_eyre::eyre::ContextCompat;
use color_eyre::{Report, Section};
use compact_str::{CompactString, ToCompactString};
use dashmap::DashMap;
use itertools::Itertools;
use node_semver::Version;
use owo_colors::OwoColorize;
use rustc_hash::{FxHashMap, FxHashSet};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::mem::take;
use std::sync::Arc;
use tokio::task::JoinHandle;

#[derive(Deserialize, Debug, Default, Clone)]
pub struct Graph {
    #[serde(flatten)]
    pub relations: FxHashMap<DepReq, VersionedSubpackage>,
}

impl Graph {
    pub async fn append(
        &mut self,
        remaining: impl Iterator<Item = DepReq>,
        download: bool,
    ) -> color_eyre::Result<()> {
        fn queue_resolve(
            send: flume::Sender<JoinHandle<color_eyre::Result<()>>>,
            req: DepReq,
            relations: Arc<DashMap<DepReq, Option<VersionedSubpackage>>>,
            download: bool,
        ) -> color_eyre::Result<()> {
            if relations.contains_key(&req) {
                return Ok(());
            }

            relations.insert(req.clone(), None);

            send.clone().send(tokio::spawn(async move {
                let (version, subpackage) = npm::fetch_dep_single(req.clone()).await?;

                if download && subpackage.supported() {
                    tokio::spawn(download_package_shared(Dependency {
                        name: req.name.to_compact_string(),
                        version: version.clone(),
                        dist: subpackage.dist.clone(),
                        bins: subpackage.bins().into_iter().collect(),
                        scripts: subpackage.scripts.clone(),
                    }));
                }

                relations.insert(
                    req.clone(),
                    Some(VersionedSubpackage {
                        package: subpackage.clone(),
                        version,
                    }),
                );

                for child_req in subpackage.iter() {
                    queue_resolve(send.clone(), child_req, relations.clone(), download)?;
                }

                Ok(()) as color_eyre::Result<_>
            }))?;

            Ok(())
        }

        let relations: Arc<DashMap<_, _>> = Arc::new(
            take(&mut self.relations)
                .into_iter()
                .map(|x| (x.0, Some(x.1)))
                .collect(),
        );

        let (send, recv) = flume::unbounded();

        for req in remaining {
            queue_resolve(send.clone(), req, relations.clone(), download)?;
        }

        drop(send);

        while let Ok(f) = recv.recv_async().await {
            f.await??;
        }

        self.relations = relations
            .iter()
            .map(|x| (x.key().clone(), x.value().clone().unwrap()))
            .collect();

        Ok(())
    }

    fn resolve_req(&self, req: &DepReq) -> color_eyre::Result<VersionedSubpackage, Report> {
        Ok(self
            .relations
            .get(req)
            .wrap_err("A dependency could not be found")
            .with_note(|| format!("Attempted to find {req:?}"))
            .with_suggestion(|| {
                if ARGS.immutable {
                    "Make sure that the lockfile is up-to-date. Passing --immutable prevents any changes to the lockfile."
                } else {
                    "Make sure that the lockfile is consistent. Automatic resolution of merge conflicts can lead to inconsistency."
                }
            })?
            .clone())
    }

    fn build_tree(
        &self,
        package: &VersionedSubpackage,
        stack: &mut Vec<VersionedSubpackage>,
        exclude: &FxHashSet<(CompactString, Version)>,
        optional: bool,
    ) -> color_eyre::Result<Option<DependencyTree>> {
        if stack.iter().any(|x| package == x) {
            log_verbose(&format!(
                "Detected cyclic dependencies: {} > {} {}",
                stack
                    .iter()
                    .map(
                        |package| format!("{}@{}", package.package.name, package.version)
                            .bright_blue()
                            .to_string()
                    )
                    .join(" > "),
                package.package.name,
                package.version
            ));

            return Ok(None);
        }

        let root = Dependency {
            name: package.package.name.to_compact_string(),
            version: package.version.clone(),
            dist: package.package.dist.clone(),
            bins: package.package.bins().into_iter().collect(),
            scripts: package.package.scripts.clone(),
        };

        if !package.package.supported() {
            if optional {
                return Ok(None);
            } else {
                return Err(
                    Report::msg("Required dependency is not supported").note(format!(
                        "Package {}@{} is not supported on this platform.",
                        package.package.name, package.version
                    )),
                );
            }
        }

        let mut deps = vec![];
        for dep in package.package.iter() {
            let package2 = self.resolve_req(&dep)?;
            stack.push(package.clone());
            if !exclude.contains(&(package2.package.name.clone(), package2.version.clone())) {
                if let Some(tree) = self.build_tree(&package2, stack, exclude, dep.optional)? {
                    deps.push(tree);
                }
            }
            stack.pop().unwrap();
        }

        let tree = DependencyTree {
            children: deps
                .into_iter()
                .map(|x| (x.root.name.to_compact_string(), x))
                .collect(),
            root,
        };

        Ok(Some(tree))
    }

    pub fn build_trees(&self, root_reqs: &[DepReq]) -> color_eyre::Result<Vec<DependencyTree>> {
        let mut is_optional = FxHashMap::default();

        let mut reqs = FxHashMap::default();

        for req in root_reqs {
            let pkg = self.resolve_req(req)?;
            reqs.insert(req.name.clone(), pkg.clone());
            is_optional.insert(pkg, req.optional);
        }

        let mut flat_deps = FxHashSet::default();
        let mut edge = VecDeque::new();
        edge.extend(reqs.values().cloned());

        while let Some(next) = edge.pop_front() {
            if !flat_deps.contains(&next) {
                for req in next.package.iter() {
                    let pkg = self.resolve_req(&req)?;
                    is_optional.insert(pkg.clone(), req.optional);
                    edge.push_back(pkg);
                }
                flat_deps.insert(next);
            }
        }

        let mut hoisted: FxHashMap<_, VersionedSubpackage> = FxHashMap::default();
        for dep in flat_deps {
            if let Some(prev) = hoisted.get(&dep.package.name) {
                if dep.version > prev.version {
                    hoisted.insert(dep.package.name.clone(), dep.clone());
                }
            } else {
                hoisted.insert(dep.package.name.clone(), dep.clone());
            }
        }

        for (name, pkg) in &reqs {
            hoisted.insert(name.clone(), pkg.clone());
        }

        for (name, pkg) in hoisted.iter() {
            reqs.insert(name.clone(), pkg.clone());
        }

        let exclude = hoisted
            .into_iter()
            .map(|(name, pkg)| (name, pkg.version))
            .collect();

        let mut v = vec![];
        for pkg in reqs.values() {
            v.push(self.build_tree(pkg, &mut vec![], &exclude, is_optional[pkg])?);
        }

        let v = v.into_iter().flatten().collect();
        Ok(v)
    }
}

#[derive(Serialize, Deserialize, Default)]
pub struct Lockfile {
    #[serde(flatten)]
    pub relations: BTreeMap<DepReq, (Version, Subpackage)>,
}

impl Lockfile {
    pub fn new(graph: Graph) -> Self {
        Self {
            relations: graph
                .relations
                .into_iter()
                .map(|(req, pkg)| (req, (pkg.version, (*pkg.package).clone())))
                .collect(),
        }
    }

    pub fn into_graph(self) -> Graph {
        Graph {
            relations: self
                .relations
                .into_iter()
                .map(|(req, pkg)| {
                    (
                        req,
                        VersionedSubpackage {
                            package: Arc::new(pkg.1),
                            version: pkg.0,
                        },
                    )
                })
                .collect(),
        }
    }
}
