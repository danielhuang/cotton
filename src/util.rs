use color_eyre::eyre::{Context, Result};
use color_eyre::Report;
use compact_str::{CompactString, ToCompactString};
use node_semver::{Range, Version};
use once_cell::sync::Lazy;
use reqwest::{Client, ClientBuilder, Url};
use serde::de::DeserializeOwned;
use serde::{de::Error, Deserialize, Serialize};
use serde_json::Value;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::{
    env::consts::{ARCH, OS},
    fmt::Display,
};
use tokio::fs::{read_to_string, File, OpenOptions};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tracing::instrument;

use crate::package::Package;
use crate::progress::log_warning;
use crate::resolve::{Graph, Lockfile};

pub const CLIENT_LIMIT: usize = 100;

pub static CLIENT: Lazy<Client> = Lazy::new(Client::new);
pub static CLIENT_Z: Lazy<Client> = Lazy::new(|| {
    ClientBuilder::new()
        .brotli(true)
        .gzip(true)
        .deflate(true)
        .build()
        .unwrap()
});

pub fn decode_json<T: DeserializeOwned>(
    x: &[u8],
) -> Result<T, serde_path_to_error::Error<serde_json::Error>> {
    let jd = &mut serde_json::Deserializer::from_slice(x);

    serde_path_to_error::deserialize(jd)
}

#[derive(Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct PrefixedVersionReq {
    pub prefix: CompactString,
    pub rest: CompactString,
}

impl Display for PrefixedVersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.prefix, self.rest)
    }
}

impl Serialize for PrefixedVersionReq {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.to_string().serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for PrefixedVersionReq {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let (prefix, rest) = s
            .split_once(':')
            .ok_or_else(|| D::Error::custom("missing :"))?;
        if prefix.starts_with("http") {
            return Err(D::Error::custom("http not allowed"));
        }
        Ok(Self {
            prefix: prefix.to_compact_string(),
            rest: rest.to_compact_string(),
        })
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
#[serde(untagged)]
pub enum VersionReq {
    Range(Range),
    Prefixed(PrefixedVersionReq),
    DirectUrl(Url),
    Other(CompactString),
}

impl VersionReq {
    pub fn satisfies(&self, v: &Version) -> bool {
        match self {
            VersionReq::Range(r) => r.satisfies(v),
            VersionReq::Prefixed(_) => true,
            VersionReq::DirectUrl(_) => true,
            VersionReq::Other(_) => false,
        }
    }
}

impl Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionReq::Range(a) => a.fmt(f),
            VersionReq::Prefixed(a) => a.fmt(f),
            VersionReq::DirectUrl(a) => a.fmt(f),
            VersionReq::Other(a) => a.fmt(f),
        }
    }
}

pub fn get_node_os() -> &'static str {
    match OS {
        "linux" => "linux",
        "macos" => "darwin",
        "freebsd" => "freebsd",
        "openbsd" => "openbsd",
        "windows" => "win32",
        _ => unreachable!(),
    }
}

pub fn get_node_cpu() -> &'static str {
    match ARCH {
        "x86_64" => "x64",
        x => x,
    }
}

const RETRY_LIMIT: usize = 3;

pub async fn retry<T, Fut: Future<Output = Result<T>>>(mut f: impl FnMut() -> Fut) -> Result<T> {
    let mut last = None;
    for _ in 0..RETRY_LIMIT {
        match f().await {
            Ok(x) => return Ok(x),
            Err(e) => {
                log_warning(&format!("Retrying {e}"));
                last = Some(e);
            }
        }
    }
    Err(last.unwrap()).wrap_err("Failed all retries")
}

pub async fn read_package() -> Result<Package> {
    read_json("package.json").await
}

pub async fn read_or_create_package<T: DeserializeOwned>() -> Result<T> {
    let mut s = String::new();
    OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open("package.json")
        .await?
        .read_to_string(&mut s)
        .await?;
    Ok(serde_json::from_str(&s)?)
}

pub async fn save_package(package: &Value) -> Result<()> {
    write_json("package.json", package).await
}

#[instrument]
pub async fn read_json<T: DeserializeOwned>(path: impl AsRef<Path> + std::fmt::Debug) -> Result<T> {
    Ok(serde_json::from_str(&read_to_string(path).await?)?)
}

pub async fn write_json<T: Serialize>(path: impl AsRef<Path>, data: T) -> Result<()> {
    let mut file = File::create(path).await?;

    file.write_all(serde_json::to_string_pretty(&data)?.as_bytes())
        .await?;

    file.flush().await?;

    Ok(())
}

pub async fn load_graph_from_lockfile() -> Graph {
    let lockfile: Lockfile = read_json("cotton.lock").await.unwrap_or_default();
    lockfile.into_graph()
}

pub type ArcResult<T, E = Report> = Result<T, Arc<E>>;
