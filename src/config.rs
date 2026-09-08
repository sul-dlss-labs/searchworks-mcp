use std::{env, net::SocketAddr, time::Duration};

use anyhow::{Context, Result};
use url::Url;

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_address: SocketAddr,
    pub searchworks_base_url: Url,
    pub request_timeout: Duration,
    pub max_response_bytes: usize,
    pub user_agent: String,
    pub mcp_allowed_hosts: Vec<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_address = env::var("BIND_ADDRESS")
            .unwrap_or_else(|_| "0.0.0.0:3000".into())
            .parse()
            .context("BIND_ADDRESS must be a host:port socket address")?;
        let searchworks_base_url = env::var("SEARCHWORKS_BASE_URL")
            .unwrap_or_else(|_| "https://searchworks.stanford.edu".into())
            .parse::<Url>()
            .context("SEARCHWORKS_BASE_URL must be a valid URL")?;
        if searchworks_base_url.scheme() != "https"
            && searchworks_base_url.host_str() != Some("localhost")
            && searchworks_base_url.host_str() != Some("127.0.0.1")
        {
            anyhow::bail!("SEARCHWORKS_BASE_URL must use HTTPS except for localhost");
        }

        Ok(Self {
            bind_address,
            searchworks_base_url,
            request_timeout: Duration::from_secs(parse_env("REQUEST_TIMEOUT_SECONDS", 15)?),
            max_response_bytes: parse_env("MAX_RESPONSE_BYTES", 2_000_000)?,
            user_agent: env::var("UPSTREAM_USER_AGENT")
                .unwrap_or_else(|_| format!("searchworks-mcp/{}", env!("CARGO_PKG_VERSION"))),
            mcp_allowed_hosts: comma_separated_env(
                "MCP_ALLOWED_HOSTS",
                "localhost,127.0.0.1,::1,searchworks-mcp",
            ),
        })
    }
}

fn comma_separated_env(name: &str, default: &str) -> Vec<String> {
    env::var(name)
        .unwrap_or_else(|_| default.into())
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .collect()
}

fn parse_env<T>(name: &str, default: T) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(name) {
        Ok(value) => value
            .parse()
            .with_context(|| format!("{name} has an invalid value")),
        Err(_) => Ok(default),
    }
}
