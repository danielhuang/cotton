use std::future::Future;
use std::{
    env::consts::{ARCH, OS},
    fmt::Display,
};

use color_eyre::eyre::Result;
use compact_str::CompactString;
use node_semver::{Range, Version};
use once_cell::sync::Lazy;
use reqwest::{Client, ClientBuilder};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::progress::log_warning;

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

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(untagged)]
pub enum VersionReq {
    Range(Range),
    Other(CompactString),
}

impl VersionReq {
    pub fn satisfies(&self, v: &Version) -> bool {
        match self {
            VersionReq::Range(r) => r.satisfies(v),
            VersionReq::Other(_) => false,
        }
    }
}

impl Display for VersionReq {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VersionReq::Range(a) => a.fmt(f),
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
                log_warning(&format!("Retrying {}", e));
                last = Some(e);
            }
        }
    }
    Err(last.unwrap().wrap_err("Failed all retries"))
}
