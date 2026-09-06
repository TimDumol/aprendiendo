use std::sync::Arc;

use anyhow::{Context, Result};
use aprendiendo_mcp::{
    auth::{Authenticator, protected_resource_metadata, require_auth},
    config::Config,
    db::{SharedStore, SqliteStore},
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
use tower_http::trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer};
use tracing::{Level, info, warn};
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
    let store: SharedStore = Arc::new(SqliteStore::new(&config.database_path)?);
    store.ping().await.context("initial database ping failed")?;
    let oauth_scope = match &config.auth {
        aprendiendo_mcp::config::AuthConfig::Oidc(oidc) => Some(oidc.required_scope.clone()),
        aprendiendo_mcp::config::AuthConfig::EmbeddedOauth(embed) => {
            Some(embed.required_scope.clone())
        }
        aprendiendo_mcp::config::AuthConfig::Disabled => None,
        aprendiendo_mcp::config::AuthConfig::Bearer(_) => None,
    };
    let public_host = match &config.auth {
        aprendiendo_mcp::config::AuthConfig::Oidc(oidc) => Some(&oidc.public_base_url),
        aprendiendo_mcp::config::AuthConfig::EmbeddedOauth(embed) => Some(&embed.public_base_url),
        _ => None,
    }
    .and_then(|base_url| url::Url::parse(base_url).ok())
    .and_then(|url| url.host_str().map(str::to_owned));
    let auth = Authenticator::new(config.auth.clone()).await?;
    if matches!(auth, Authenticator::Disabled) {
        warn!("authentication is disabled; do not expose this listener publicly");
    }

    let cancellation = CancellationToken::new();
    let template = LearningServer::new(store.clone(), oauth_scope);
    let mut transport_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_cancellation_token(cancellation.child_token());
    if let Some(public_host) = public_host {
        transport_config = transport_config.with_allowed_hosts([
            "localhost".to_owned(),
            "127.0.0.1".to_owned(),
            "::1".to_owned(),
            public_host,
        ]);
    }

    let mcp_service = StreamableHttpService::new(
        move || Ok(template.clone()),
        LocalSessionManager::default().into(),
        transport_config,
    );

    let protected = Router::new()
        .nest_service("/mcp", mcp_service)
        .layer(DefaultBodyLimit::max(config.max_request_bytes))
        // Keep authentication scoped to registered MCP routes. A regular
        // `layer` also wraps the router fallback, turning unknown OIDC-only
        // paths into 401s instead of the expected 404.
        .route_layer(middleware::from_fn_with_state(auth.clone(), require_auth));

    let auth_routes = Router::new()
        .route("/health", get(health))
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource_metadata),
        )
        .with_state(auth.clone());

    let mut app = Router::new()
        .merge(auth_routes)
        .route("/ready", get(readiness).with_state(AppState { store }))
        .merge(protected);

    if let Authenticator::EmbeddedOauth(config_arc, state) = auth {
        let oauth_routes = Router::new()
            .route(
                "/.well-known/oauth-authorization-server",
                get(aprendiendo_mcp::embedded_oauth::authorization_server_metadata)
                    .with_state(config_arc.clone()),
            )
            .route(
                "/oauth/jwks",
                get(aprendiendo_mcp::embedded_oauth::jwks).with_state(state.clone()),
            )
            .route(
                "/oauth/authorize",
                get(aprendiendo_mcp::embedded_oauth::authorize_get)
                    .with_state(config_arc)
                    .post(aprendiendo_mcp::embedded_oauth::authorize_post)
                    .with_state(state.clone()),
            )
            .route(
                "/oauth/token",
                axum::routing::post(aprendiendo_mcp::embedded_oauth::token_post)
                    .with_state(state.clone()),
            );
        app = app.merge(oauth_routes);
    }

    // Log method, URI, status, and latency for every request. Headers and bodies
    // are deliberately excluded so credentials, cookies, and OAuth codes do not
    // end up in the application logs.
    app = app.layer(
        TraceLayer::new_for_http()
            .make_span_with(
                DefaultMakeSpan::new()
                    .level(Level::INFO)
                    .include_headers(false),
            )
            .on_response(DefaultOnResponse::new().level(Level::INFO)),
    );

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
