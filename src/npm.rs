use cached::proc_macro::cached;
use color_eyre::{
    eyre::{eyre, ContextCompat, Result},
    Report,
};
use compact_str::{CompactString, ToCompactString};
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
    path::MAIN_SEPARATOR,
    sync::Arc,
};
use tap::Pipe;
use tokio::sync::Semaphore;

use crate::{
    cache::Cache,
    config::{read_config, Registry},
    package::{DepReq, Dist, Package, Subpackage},
    progress::log_progress,
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

async fn select_registry(name: &str) -> Result<Registry> {
    for registry in read_config().await?.registry {
        if let Some(scope) = &registry.scope {
            if name.starts_with(scope) {
                return Ok(registry);
            }
        } else {
            return Ok(registry);
        }
    }

    Ok(Registry {
        url: "https://registry.npmjs.org".into(),
        scope: None,
        auth: None,
    })
}

#[tracing::instrument]
pub async fn fetch_package(name: &str) -> Result<RegistryResponse> {
    static S: Lazy<Semaphore> = Lazy::new(|| Semaphore::new(CLIENT_LIMIT));
    let _permit = S.acquire().await.unwrap();

    let selected_registry = select_registry(name).await?;

    let token = if let Some(auth) = selected_registry.auth {
        Some(auth.read_token()?)
    } else {
        None
    };

    retry(|| async {
        decode_json(
            &CLIENT_Z
                .get(format!("{}/{name}", selected_registry.url))
                .pipe(|x| {
                    if let Some(token) = &token {
                        x.bearer_auth(token)
                    } else {
                        x
                    }
                })
                .send()
                .await?
                .error_for_status()?
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
pub async fn fetch_dep_single(d: DepReq) -> Result<(Version, Arc<Subpackage>)> {
    let res = fetch_package_cached(&d.name).await?;

    log_progress(&format!("Fetched {}", d.name.bright_blue()));

    if let VersionReq::Other(tag) = &d.version {
        let tag = res
            .dist_tags
            .get(tag)
            .wrap_err_with(|| eyre!("Version cannot be satisfied: {} {}", d.name, d.version))?;
        let version = Version::parse(tag)?;
        let package = res.versions.get(&version).wrap_err_with(|| {
            eyre!(
                "Tag refers to a version that does not exist: {} - {} refers to {}",
                d.name,
                d.version,
                version
            )
        })?;

        Ok((version, Arc::new(package.clone().sub())))
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

        Ok((version.clone(), Arc::new(package.clone().sub())))
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Clone, Debug)]
pub struct DependencyTree {
    #[serde(flatten)]
    pub root: Dependency,
    pub children: FxHashMap<CompactString, DependencyTree>,
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
                        Some((name, tree.filter(exclude)))
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
        format!("{}@{}", self.name, self.version).replace(MAIN_SEPARATOR, "!")
    }
}
