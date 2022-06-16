use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fmt::Debug,
};

use color_eyre::eyre::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    fs::{read_to_string, File},
    io::AsyncWriteExt,
};

use crate::{npm::PlatformMap, util::PartialRange};

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Package {
    pub name: Option<String>,
    pub bin: Option<Bin>,
    pub dist: Dist,
    pub dependencies: HashMap<String, PartialRange>,
    pub optional_dependencies: HashMap<String, PartialRange>,
    pub dev_dependencies: HashMap<String, PartialRange>,
    pub os: PlatformMap,
    pub cpu: PlatformMap,
    pub scripts: HashMap<String, String>,
    #[serde(flatten)]
    pub rest: Value,
}

impl Package {
    pub fn bins(&self) -> BTreeMap<String, String> {
        match &self.bin {
            Some(Bin::Multi(x)) => x.clone().into_iter().collect(),
            Some(Bin::Single(x)) => {
                if let Some(name) = &self.name {
                    [(name.to_string(), x.to_string())].into_iter().collect()
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
    Single(String),
    Multi(HashMap<String, String>),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub struct Dist {
    pub tarball: String,
    pub unpacked_size: Option<u64>,
}

#[derive(PartialEq, Eq, Hash, Clone)]
pub struct DepReq {
    pub name: String,
    pub version: PartialRange,
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
            name: n.to_string(),
            version: v.to_owned(),
            optional: self.optional_dependencies.contains_key(n),
        })
    }

    pub fn iter_with_dev(&self) -> impl Iterator<Item = DepReq> + '_ {
        self.dependencies
            .iter()
            .map(|(n, v)| DepReq {
                name: n.to_string(),
                version: v.to_owned(),
                optional: self.optional_dependencies.contains_key(n),
            })
            .chain(self.dev_dependencies.iter().map(|(n, v)| DepReq {
                name: n.to_string(),
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
    File::create("package.json")
        .await?
        .write_all(serde_json::to_string_pretty(package)?.as_bytes())
        .await?;

    Ok(())
}
