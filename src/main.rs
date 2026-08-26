use std::sync::Arc;

use anyhow::{Context, Result};
use aprendiendo_mcp::{
    auth::{Authenticator, protected_resource_metadata, require_auth},
    config::Config,
    db::{PostgresStore, SharedStore},
    server::LearningServer,
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    middleware,
    routing::get,
};
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    store: SharedStore,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "aprendiendo_mcp=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = Config::from_env()?;
    let store: SharedStore = Arc::new(PostgresStore::new(
        config.database_url.clone(),
        config.database_pool_size,
        config.database_timeout,
    )?);
    store.ping().await.context("initial database ping failed")?;
    let oauth_scope = match &config.auth {
        aprendiendo_mcp::config::AuthConfig::Oidc(oidc) => Some(oidc.required_scope.clone()),
        aprendiendo_mcp::config::AuthConfig::Disabled => None,
        aprendiendo_mcp::config::AuthConfig::Bearer(_) => None,
    };
    let auth = Authenticator::new(config.auth.clone()).await?;
    if matches!(auth, Authenticator::Disabled) {
        warn!("authentication is disabled; do not expose this listener publicly");
    }

    let cancellation = CancellationToken::new();
    let template = LearningServer::new(store.clone(), oauth_scope);
    let mcp_service = StreamableHttpService::new(
        move || Ok(template.clone()),
        LocalSessionManager::default().into(),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true)
            .with_cancellation_token(cancellation.child_token()),
    );

    let protected = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(DefaultBodyLimit::max(config.max_request_bytes))
        .layer(middleware::from_fn_with_state(auth.clone(), require_auth));
    let auth_routes = Router::new()
        .route("/health", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .with_state(auth);
    let readiness_route = Router::new()
        .route("/ready", get(readiness))
        .with_state(AppState { store });
    let app = Router::new()
        .merge(auth_routes)
        .merge(readiness_route)
        .merge(protected);

    let listener = tokio::net::TcpListener::bind(config.bind_addr)
        .await
        .with_context(|| format!("failed to bind {}", config.bind_addr))?;
    info!(address = %config.bind_addr, "Aprendiendo MCP listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown(cancellation))
        .await
        .context("HTTP server failed")?;
    Ok(())
}

async fn health() -> Json<serde_json::Value> {
    Json(json!({"status": "ok"}))
}

async fn readiness(
    State(state): State<AppState>,
) -> (axum::http::StatusCode, Json<serde_json::Value>) {
    match state.store.ping().await {
        Ok(()) => (axum::http::StatusCode::OK, Json(json!({"status": "ready"}))),
        Err(_) => (
            axum::http::StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"status": "unavailable"})),
        ),
    }
}

async fn shutdown(cancellation: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
    cancellation.cancel();
}
