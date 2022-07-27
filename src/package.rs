use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
};

use crate::{npm::PlatformMap, util::VersionReq};
use color_eyre::eyre::Result;
use compact_str::{CompactString, ToCompactString};
use rustc_hash::FxHashMap;
use serde::{
    de::{self},
    Deserialize, Serialize,
};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Package {
    pub name: Option<CompactString>,
    pub bin: Option<Bin>,
    pub dist: Dist,
    pub dependencies: BTreeMap<CompactString, VersionReq>,
    pub optional_dependencies: BTreeMap<CompactString, VersionReq>,
    pub dev_dependencies: FxHashMap<CompactString, VersionReq>,
    pub os: PlatformMap,
    pub cpu: PlatformMap,
    pub scripts: FxHashMap<CompactString, Value>,
}

impl Package {
    pub fn sub(self) -> Subpackage {
        Subpackage {
            name: self.name,
            dist: self.dist,
            dependencies: self.dependencies,
            optional_dependencies: self.optional_dependencies,
            os: self.os,
            cpu: self.cpu,
            bin: self.bin,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct Subpackage {
    pub name: Option<CompactString>,
    pub dist: Dist,
    pub dependencies: BTreeMap<CompactString, VersionReq>,
    pub optional_dependencies: BTreeMap<CompactString, VersionReq>,
    pub os: PlatformMap,
    pub cpu: PlatformMap,
    pub bin: Option<Bin>,
}

impl Subpackage {
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

    pub fn iter(&self) -> impl Iterator<Item = DepReq> + '_ {
        self.dependencies.iter().map(|(n, v)| DepReq {
            name: n.to_compact_string(),
            version: v.to_owned(),
            optional: self.optional_dependencies.contains_key(n),
        })
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

impl Display for DepReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}!{}{}",
            self.name,
            self.version,
            if self.optional { "?" } else { "" }
        )
    }
}

impl Serialize for DepReq {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DepReq {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let (name, rest) = s
            .split_once('!')
            .ok_or_else(|| de::Error::custom("Failed to parse version"))?;
        let optional = rest.ends_with('?');
        let version = rest.trim_end_matches('?');
        Ok(Self {
            name: name.to_compact_string(),
            version: serde_json::from_value(Value::String(version.to_string()))
                .map_err(de::Error::custom)?,
            optional,
        })
    }
}

impl PartialOrd for DepReq {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        self.to_string().partial_cmp(&other.to_string())
    }
}

impl Ord for DepReq {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.to_string().cmp(&other.to_string())
    }
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
