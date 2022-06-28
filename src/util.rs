use std::{
    env::consts::{ARCH, OS},
    fmt::Display,
};

use compact_str::CompactString;
use node_semver::{Range, Version};
use once_cell::sync::Lazy;
use reqwest::{Client, ClientBuilder};
use serde::{Deserialize, Serialize};

pub static CLIENT: Lazy<Client> = Lazy::new(Client::new);
pub static CLIENT2: Lazy<Client> = Lazy::new(|| {
    ClientBuilder::new()
        .http2_prior_knowledge()
        .brotli(true)
        .gzip(true)
        .deflate(true)
        .build()
        .unwrap()
});

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(untagged)]
pub enum PartialRange {
    Range(Range),
    Oops(CompactString),
}

impl PartialRange {
    pub fn satisfies(&self, v: &Version) -> bool {
        match self {
            PartialRange::Range(r) => r.satisfies(v),
            PartialRange::Oops(s) => {
                println!("ops: {}", s);
                false
            }
        }
    }
}

impl Display for PartialRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartialRange::Range(a) => a.fmt(f),
            PartialRange::Oops(a) => a.fmt(f),
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
