use cached::proc_macro::cached;
use color_eyre::eyre::Result;
use serde::{Deserialize, Serialize};
use std::env;
use tokio::fs::read_to_string;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Config {
    #[serde(default)]
    pub registry: Vec<Registry>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
pub struct Registry {
    pub url: String,
    pub scope: Option<String>,
    pub auth: Option<RegistryAuth>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
pub enum RegistryAuth {
    Token(String),
    FromEnv(String),
}

impl RegistryAuth {
    #[tracing::instrument]
    pub fn read_token(&self) -> Result<String> {
        match self {
            RegistryAuth::Token(x) => Ok(x.clone()),
            RegistryAuth::FromEnv(x) => Ok(env::var(x)?),
        }
    }
}

#[cached(result)]
pub async fn read_config() -> Result<Config> {
    let config = read_to_string("cotton.toml").await;
    if let Ok(config) = config {
        Ok(toml::from_str(&config)?)
    } else {
        Ok(Config::default())
    }
}
