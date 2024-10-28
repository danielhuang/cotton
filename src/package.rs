use std::{
    collections::BTreeMap,
    fmt::{Debug, Display},
    sync::Arc,
};

use crate::{
    npm::PlatformMap,
    util::{get_node_cpu, get_node_os, VersionSpecifier},
};
use color_eyre::eyre::Result;
use compact_str::{CompactString, ToCompactString};
use node_semver::Version;
use rustc_hash::FxHashMap;
use serde::{
    de::{self},
    Deserialize, Serialize,
};
use serde_json::Value;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct PackageMetadata {
    pub name: CompactString,
    pub version: Option<Version>,
    pub bin: Option<Bin>,
    pub dist: Dist,
    pub dependencies: BTreeMap<CompactString, VersionSpecifier>,
    pub optional_dependencies: BTreeMap<CompactString, VersionSpecifier>,
    pub dev_dependencies: FxHashMap<CompactString, VersionSpecifier>,
    pub os: PlatformMap,
    pub cpu: PlatformMap,
    pub scripts: FxHashMap<CompactString, Value>,
}

impl PackageMetadata {
    pub fn info(self) -> PackageInfo {
        PackageInfo {
            name: self.name,
            dist: self.dist,
            dependencies: self.dependencies,
            optional_dependencies: self.optional_dependencies,
            os: self.os,
            cpu: self.cpu,
            bin: self.bin,
            scripts: self
                .scripts
                .iter()
                .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_compact_string())))
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default, Hash)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct PackageInfo {
    pub name: CompactString,
    pub dist: Dist,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub dependencies: BTreeMap<CompactString, VersionSpecifier>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub optional_dependencies: BTreeMap<CompactString, VersionSpecifier>,
    #[serde(skip_serializing_if = "PlatformMap::is_empty")]
    pub os: PlatformMap,
    #[serde(skip_serializing_if = "PlatformMap::is_empty")]
    pub cpu: PlatformMap,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bin: Option<Bin>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scripts: BTreeMap<CompactString, CompactString>,
}

#[derive(Debug, Hash, PartialEq, Eq, Clone, Deserialize)]
pub struct VersionedPackageInfo {
    pub package: Arc<PackageInfo>,
    pub version: Version,
}

impl PackageInfo {
    pub fn bins(&self) -> BTreeMap<CompactString, CompactString> {
        match &self.bin {
            Some(Bin::Multi(x)) => x.clone().into_iter().collect(),
            Some(Bin::Single(x)) => [(self.name.to_compact_string(), x.to_compact_string())]
                .into_iter()
                .collect(),
            None => [].into_iter().collect(),
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = PackageSpecifier> + '_ {
        self.dependencies
            .iter()
            .chain(self.optional_dependencies.iter())
            .map(|(n, v)| PackageSpecifier {
                name: n.to_compact_string(),
                version: v.to_owned(),
                optional: self.optional_dependencies.contains_key(n),
            })
    }

    pub fn supported(&self) -> bool {
        self.os.is_supported(get_node_os()) && self.cpu.is_supported(get_node_cpu())
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Hash)]
#[serde(untagged)]
pub enum Bin {
    Single(CompactString),
    Multi(BTreeMap<CompactString, CompactString>),
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default, PartialOrd, Ord)]
pub struct Dist {
    pub tarball: CompactString,
}

#[derive(PartialEq, Eq, Hash, Clone, PartialOrd, Ord)]
pub struct PackageSpecifier {
    pub name: CompactString,
    pub version: VersionSpecifier,
    pub optional: bool,
}

impl Display for PackageSpecifier {
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

impl Serialize for PackageSpecifier {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PackageSpecifier {
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

impl Debug for PackageSpecifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} {}", self.name, self.version)?;
        if self.optional {
            write!(f, " (optional)")?;
        }
        Ok(())
    }
}

impl PackageMetadata {
    pub fn iter_all(&self) -> impl Iterator<Item = PackageSpecifier> + '_ {
        self.dependencies
            .iter()
            .chain(self.dev_dependencies.iter())
            .chain(self.optional_dependencies.iter())
            .map(|(n, v)| PackageSpecifier {
                name: n.to_compact_string(),
                version: v.to_owned(),
                optional: self.optional_dependencies.contains_key(n),
            })
    }
}
