use anyhow::{Context, Result};
use aprendiendo_mcp::practice_mvp::{self, MvpConfig};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // This binary intentionally loads only the practice MVP's local env files.
    // It does not call the production Config::from_env and never opens SQLite.
    // Keep the key out of EXPO_PUBLIC_* variables: the Rust process is the
    // only part of this app that should receive it.
    for filename in [
        ".env.mvp",
        ".env.mcp",
        ".env",
        "apps/practice/.env.mvp",
        "apps/practice/.env.mcp",
        "apps/practice/.env",
    ] {
        dotenvy::from_filename(filename).ok();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "practice_mvp=info,aprendiendo_mcp::practice_mvp=info".into()),
        )
        .init();

    let config = MvpConfig::from_env()?;
    let app = practice_mvp::router(config).context("failed to build the practice MVP router")?;
    let listener_address = practice_mvp::listener_address();
    let listener = tokio::net::TcpListener::bind(&listener_address)
        .await
        .with_context(|| format!("failed to bind {}", listener_address))?;
    info!(
        address = %listener_address,
        "practice MVP listening"
    );
    axum::serve(listener, app)
        .await
        .context("practice MVP HTTP server failed")?;
    Ok(())
}
