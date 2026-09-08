use std::time::Duration;

use axum::{Router, routing::get};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use searchworks_mcp::{client::SearchworksClient, config::Config, server::SearchworksMcp};
use tokio_util::sync::CancellationToken;
use tower_http::{
    catch_panic::CatchPanicLayer,
    request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
    trace::TraceLayer,
};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "searchworks_mcp=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer().json())
        .init();
    let config = Config::from_env()?;
    let bind_address = config.bind_address;
    let allowed_hosts = config.mcp_allowed_hosts.clone();
    let client = SearchworksClient::new(&config)?;
    let cancellation = CancellationToken::new();
    let service = StreamableHttpService::new(
        move || Ok(SearchworksMcp::new(client.clone())),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_allowed_hosts(allowed_hosts)
            .with_cancellation_token(cancellation.child_token()),
    );
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/mcp", service)
        .layer(PropagateRequestIdLayer::x_request_id())
        .layer(SetRequestIdLayer::new(
            http::header::HeaderName::from_static("x-request-id"),
            MakeRequestUuid,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(CatchPanicLayer::new());
    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    tracing::info!(%bind_address, "searchworks-mcp listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(cancellation))
        .await?;
    Ok(())
}

async fn shutdown(cancellation: CancellationToken) {
    let _ = tokio::signal::ctrl_c().await;
    cancellation.cancel();
    tokio::time::sleep(Duration::from_millis(100)).await;
}
