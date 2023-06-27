use color_eyre::eyre::Result;
use reqwest::RequestBuilder;
use serde::{Deserialize, Serialize};
use std::env;
use tokio::fs::read_to_string;

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug, Default)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub registry: Vec<Registry>,
    #[serde(default)]
    pub allow_install_scripts: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(deny_unknown_fields)]
pub struct Registry {
    pub url: String,
    pub scope: Option<String>,
    pub auth: Option<RegistryAuth>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum RegistryAuth {
    Token {
        token: AuthSource,
    },
    Basic {
        username: AuthSource,
        #[serde(default)]
        password: Option<AuthSource>,
    },
}

pub fn client_auth(req: RequestBuilder, auth: Option<&RegistryAuth>) -> Result<RequestBuilder> {
    Ok(match auth {
        Some(RegistryAuth::Token { token }) => {
            let token = token.read_token()?;
            req.bearer_auth(token)
        }
        Some(RegistryAuth::Basic { username, password }) => {
            let username = username.read_token()?;
            let password = password.as_ref().map(|x| x.read_token()).transpose()?;
            req.basic_auth(username, password)
        }
        None => req,
    })
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Hash, Debug)]
#[serde(rename_all = "snake_case")]
#[serde(untagged)]
#[serde(deny_unknown_fields)]
pub enum AuthSource {
    Inline(String),
    FromEnv { from_env: String },
}

impl AuthSource {
    #[tracing::instrument]
    pub fn read_token(&self) -> Result<String> {
        match self {
            AuthSource::Inline(x) => Ok(x.clone()),
            AuthSource::FromEnv { from_env } => Ok(env::var(from_env)?),
        }
    }
}

pub async fn read_config() -> Result<Config> {
    let config = read_to_string("cotton.toml").await;
    if let Ok(config) = config {
        Ok(toml::from_str(&config)?)
    } else {
        Ok(Config::default())
    }
}
