use reqwest::StatusCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("SearchWorks request failed")]
    Request(#[from] reqwest::Error),
    #[error("SearchWorks returned HTTP {0}")]
    Status(StatusCode),
    #[error("SearchWorks returned an unexpected content type")]
    ContentType,
    #[error("SearchWorks response exceeded the configured size limit")]
    TooLarge,
    #[error("SearchWorks returned an unexpected response")]
    InvalidResponse,
}

/// What an MCP client should do about a failure. Reporting everything as
/// "temporarily unavailable" invites an agent to retry a request that can
/// never succeed, such as a lookup of an id that does not exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The record does not exist. Retrying is pointless; the id has to change.
    NotFound,
    /// Upstream is throttling us. The same request may succeed after a wait.
    RateLimited,
    /// Upstream rejected the request itself. The arguments have to change.
    BadRequest,
    /// A fault on our side or upstream's. The same request may succeed later.
    Unavailable,
}

impl ErrorKind {
    pub fn retryable(self) -> bool {
        matches!(self, Self::RateLimited | Self::Unavailable)
    }
}

impl ApiError {
    pub fn kind(&self) -> ErrorKind {
        let Self::Status(status) = self else {
            // Timeouts, connection failures, oversized bodies and unparseable
            // payloads are all transient from the caller's point of view.
            return ErrorKind::Unavailable;
        };
        match *status {
            StatusCode::NOT_FOUND => ErrorKind::NotFound,
            StatusCode::TOO_MANY_REQUESTS => ErrorKind::RateLimited,
            // An auth failure or a bot challenge is a deployment problem, not
            // something the caller can fix by changing its arguments.
            StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ErrorKind::Unavailable,
            status if status.is_client_error() => ErrorKind::BadRequest,
            _ => ErrorKind::Unavailable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_upstream_statuses() {
        let kind = |status| ApiError::Status(status).kind();
        assert_eq!(kind(StatusCode::NOT_FOUND), ErrorKind::NotFound);
        assert_eq!(kind(StatusCode::TOO_MANY_REQUESTS), ErrorKind::RateLimited);
        assert_eq!(kind(StatusCode::BAD_REQUEST), ErrorKind::BadRequest);
        assert_eq!(kind(StatusCode::URI_TOO_LONG), ErrorKind::BadRequest);
        assert_eq!(kind(StatusCode::UNAUTHORIZED), ErrorKind::Unavailable);
        assert_eq!(kind(StatusCode::FORBIDDEN), ErrorKind::Unavailable);
        assert_eq!(
            kind(StatusCode::INTERNAL_SERVER_ERROR),
            ErrorKind::Unavailable
        );
        assert_eq!(kind(StatusCode::BAD_GATEWAY), ErrorKind::Unavailable);
    }

    #[test]
    fn classifies_transport_failures_as_unavailable() {
        assert_eq!(ApiError::ContentType.kind(), ErrorKind::Unavailable);
        assert_eq!(ApiError::TooLarge.kind(), ErrorKind::Unavailable);
        assert_eq!(ApiError::InvalidResponse.kind(), ErrorKind::Unavailable);
    }

    #[test]
    fn only_transient_kinds_are_retryable() {
        assert!(ErrorKind::RateLimited.retryable());
        assert!(ErrorKind::Unavailable.retryable());
        assert!(!ErrorKind::NotFound.retryable());
        assert!(!ErrorKind::BadRequest.retryable());
    }
}
