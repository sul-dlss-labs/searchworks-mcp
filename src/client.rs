use std::collections::BTreeMap;

use reqwest::{Client, Response, header};
use serde_json::Value;
use url::Url;

use crate::{config::Config, error::ApiError};

#[derive(Clone)]
pub struct SearchworksClient {
    client: Client,
    base_url: Url,
    max_response_bytes: usize,
}

impl SearchworksClient {
    pub fn new(config: &Config) -> anyhow::Result<Self> {
        let client = Client::builder()
            .timeout(config.request_timeout)
            .user_agent(&config.user_agent)
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Ok(Self {
            client,
            base_url: config.searchworks_base_url.clone(),
            max_response_bytes: config.max_response_bytes,
        })
    }

    pub async fn catalog_search(
        &self,
        query: &str,
        field: &str,
        rows: u8,
        filters: &BTreeMap<String, String>,
    ) -> Result<Value, ApiError> {
        let mut url = self
            .base_url
            .join("catalog.json")
            .map_err(|_| ApiError::InvalidResponse)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", query)
                .append_pair("search_field", field)
                .append_pair("per_page", &rows.to_string());
            for (field, value) in filters {
                pairs.append_pair(&format!("f[{field}][]"), value);
            }
        }
        self.get_json(url, false).await
    }

    pub async fn catalog_record(&self, id: &str) -> Result<Value, ApiError> {
        let mut url = self
            .base_url
            .join("view/")
            .map_err(|_| ApiError::InvalidResponse)?;
        url.path_segments_mut()
            .map_err(|_| ApiError::InvalidResponse)?
            .push(id);
        url.query_pairs_mut().append_pair("format", "json");
        self.get_json(url, false).await
    }

    pub async fn article_search(
        &self,
        query: &str,
        field: &str,
        rows: u8,
    ) -> Result<Value, ApiError> {
        let mut url = self
            .base_url
            .join("articles")
            .map_err(|_| ApiError::InvalidResponse)?;
        url.query_pairs_mut()
            .append_pair("format", "json")
            .append_pair("guest", "true")
            .append_pair("q", query)
            .append_pair("search_field", field)
            .append_pair("per_page", &rows.to_string());
        self.get_json(url, true).await
    }

    pub async fn article_record(&self, id: &str) -> Result<Value, ApiError> {
        let mut url = self
            .base_url
            .join("articles/")
            .map_err(|_| ApiError::InvalidResponse)?;
        url.path_segments_mut()
            .map_err(|_| ApiError::InvalidResponse)?
            .push(id);
        url.query_pairs_mut()
            .append_pair("format", "json")
            .append_pair("guest", "true");
        self.get_json(url, false).await
    }

    async fn get_json(&self, url: Url, bot_exempt: bool) -> Result<Value, ApiError> {
        tracing::debug!(uri = %redacted_uri(&url), "requesting SearchWorks");
        let mut request = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/json");
        if bot_exempt {
            request = request.header("Sec-Fetch-Dest", "empty");
        }
        let response = request.send().await?;
        self.parse_json(response).await
    }

    async fn parse_json(&self, mut response: Response) -> Result<Value, ApiError> {
        if !response.status().is_success() {
            return Err(ApiError::Status(response.status()));
        }
        let is_json = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/json"));
        if !is_json {
            return Err(ApiError::ContentType);
        }
        if response
            .content_length()
            .is_some_and(|n| n > self.max_response_bytes as u64)
        {
            return Err(ApiError::TooLarge);
        }
        let mut bytes = Vec::with_capacity(
            response
                .content_length()
                .unwrap_or(0)
                .min(self.max_response_bytes as u64) as usize,
        );
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > self.max_response_bytes {
                return Err(ApiError::TooLarge);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| ApiError::InvalidResponse)
    }
}

fn redacted_uri(url: &Url) -> String {
    let mut safe = url.clone();
    safe.set_query(None);
    safe.to_string()
}

#[cfg(test)]
mod tests {
    use std::{net::SocketAddr, time::Duration};

    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path, query_param},
    };

    use super::*;
    use crate::config::Config;

    fn client(server: &MockServer, max_response_bytes: usize) -> SearchworksClient {
        SearchworksClient::new(&Config {
            bind_address: SocketAddr::from(([127, 0, 0, 1], 3000)),
            searchworks_base_url: Url::parse(&server.uri()).expect("mock URL"),
            request_timeout: Duration::from_secs(2),
            max_response_bytes,
            user_agent: "test".into(),
            mcp_allowed_hosts: vec!["localhost".into()],
        })
        .expect("client")
    }

    #[tokio::test]
    async fn article_search_forces_guest_json_requests() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/articles"))
            .and(query_param("format", "json"))
            .and(query_param("guest", "true"))
            .and(query_param("q", "climate"))
            .and(query_param("search_field", "search"))
            .and(query_param("per_page", "3"))
            .and(header("accept", "application/json"))
            .and(header("sec-fetch-dest", "empty"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"response": {"docs": []}})),
            )
            .expect(1)
            .mount(&server)
            .await;

        client(&server, 1_000)
            .article_search("climate", "search", 3)
            .await
            .expect("valid response");
    }

    #[tokio::test]
    async fn rejects_oversized_responses_while_streaming() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw("x".repeat(101), "application/json"),
            )
            .mount(&server)
            .await;

        let result = client(&server, 100).catalog_record("123").await;
        assert!(matches!(result, Err(ApiError::TooLarge)));
    }
}
