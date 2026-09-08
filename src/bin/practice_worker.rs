use std::{env, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use aprendiendo_mcp::practice::{self, PracticeApiConfig, PracticeApiState, PracticeJobStore};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    for filename in [".env", ".env.mcp", ".env.practice"] {
        dotenvy::from_filename(filename).ok();
    }
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "practice_worker=info,aprendiendo_mcp::practice=info".into()),
        )
        .init();

    let database_path =
        env::var("DATABASE_PATH").unwrap_or_else(|_| "data/aprendiendo.sqlite3".to_owned());
    let config = Arc::new(PracticeApiConfig::from_env(&database_path)?);
    let store = Arc::new(PracticeJobStore::new(&database_path)?);
    let provider = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("build practice worker provider client")?;
    let state = PracticeApiState {
        store,
        config,
        provider,
    };
    info!(media_dir = %state.config.media_dir.display(), "practice worker started");

    loop {
        tokio::select! {
            result = practice::worker_once(&state) => {
                if result? {
                    continue;
                }
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
            _ = tokio::signal::ctrl_c() => break,
        }
    }
    Ok(())
}
