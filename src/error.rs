use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("SearchWorks request failed")]
    Request(#[from] reqwest::Error),
    #[error("SearchWorks returned HTTP {0}")]
    Status(reqwest::StatusCode),
    #[error("SearchWorks returned an unexpected content type")]
    ContentType,
    #[error("SearchWorks response exceeded the configured size limit")]
    TooLarge,
    #[error("SearchWorks returned an unexpected response")]
    InvalidResponse,
}
