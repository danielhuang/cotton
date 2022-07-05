use std::{collections::BTreeMap, fmt::Debug, path::Path};

use color_eyre::eyre::Result;
use compact_str::{CompactString, ToCompactString};
use rustc_hash::FxHashMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    fs::{read_to_string, File},
    io::AsyncWriteExt,
};

use crate::{npm::PlatformMap, util::VersionReq};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Package {
    pub name: Option<CompactString>,
    pub bin: Option<Bin>,
    pub dist: Dist,
    pub dependencies: FxHashMap<CompactString, VersionReq>,
    pub optional_dependencies: FxHashMap<CompactString, VersionReq>,
    pub dev_dependencies: FxHashMap<CompactString, VersionReq>,
    pub os: PlatformMap,
    pub cpu: PlatformMap,
    pub scripts: FxHashMap<CompactString, Value>,
    #[serde(flatten)]
    pub rest: Value,
}

impl Package {
    pub fn bins(&self) -> BTreeMap<CompactString, CompactString> {
        match &self.bin {
            Some(Bin::Multi(x)) => x.clone().into_iter().collect(),
            Some(Bin::Single(x)) => {
                if let Some(name) = &self.name {
                    [(name.to_compact_string(), x.to_compact_string())]
                        .into_iter()
                        .collect()
                } else {
                    [].into_iter().collect()
                }
            }
            None => [].into_iter().collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
#[serde(untagged)]
pub enum Bin {
    Single(CompactString),
    Multi(FxHashMap<CompactString, CompactString>),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Dist {
    pub tarball: CompactString,
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub struct DepReq {
    pub name: CompactString,
    pub version: VersionReq,
    pub optional: bool,
}

impl Debug for DepReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.version)?;
        if self.optional {
            write!(f, " (optional)")?;
        }
        Ok(())
    }
}

impl Package {
    pub fn iter(&self) -> impl Iterator<Item = DepReq> + '_ {
        self.dependencies.iter().map(|(n, v)| DepReq {
            name: n.to_compact_string(),
            version: v.to_owned(),
            optional: self.optional_dependencies.contains_key(n),
        })
    }

    pub fn iter_with_dev(&self) -> impl Iterator<Item = DepReq> + '_ {
        self.dependencies
            .iter()
            .map(|(n, v)| DepReq {
                name: n.to_compact_string(),
                version: v.to_owned(),
                optional: self.optional_dependencies.contains_key(n),
            })
            .chain(self.dev_dependencies.iter().map(|(n, v)| DepReq {
                name: n.to_compact_string(),
                version: v.to_owned(),
                optional: self.optional_dependencies.contains_key(n),
            }))
    }
}

pub async fn read_package() -> Result<Package> {
    Ok(serde_json::from_str(
        &read_to_string("package.json").await?,
    )?)
}

pub async fn read_package_as_value() -> Result<Value> {
    Ok(serde_json::from_str(
        &read_to_string("package.json").await?,
    )?)
}

pub async fn save_package(package: &Value) -> Result<()> {
    write_json("package.json", package).await
}

pub async fn write_json<T: Serialize>(path: impl AsRef<Path>, data: T) -> Result<()> {
    File::create(path)
        .await?
        .write_all(serde_json::to_string_pretty(&data)?.as_bytes())
        .await?;

    Ok(())
}
