use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use deadpool_postgres::{Config as PoolConfig, Pool, Runtime};
use serde_json::{Value, json};
use tokio::time::timeout;
use tokio_postgres_rustls::MakeRustlsConnect;

#[async_trait]
pub trait LearningStore: Send + Sync {
    async fn learning_context(&self, recent_sessions: i32) -> Result<Value>;
    async fn recent_practice(&self, limit: i32, skill: Option<&str>) -> Result<Value>;
    async fn record_practice_json(&self, payload: Value) -> Result<Value>;
    async fn review_queue(&self, limit: i32, category: Option<&str>) -> Result<Value>;
    async fn upsert_weakness_json(&self, patch: Value) -> Result<Value>;
    async fn data_status(&self) -> Result<Value>;
    async fn ping(&self) -> Result<()>;
}

#[derive(Clone)]
pub struct PostgresStore {
    pool: Pool,
    timeout: Duration,
}

impl PostgresStore {
    pub fn new(database_url: String, max_size: usize, query_timeout: Duration) -> Result<Self> {
        let mut config = PoolConfig::new();
        config.url = Some(database_url);
        config.pool = Some(deadpool_postgres::PoolConfig::new(max_size));

        rustls::crypto::ring::default_provider()
            .install_default()
            .map_err(|_| anyhow!("failed to install the process TLS provider"))?;
        let tls = MakeRustlsConnect::with_webpki_roots();
        let pool = config
            .create_pool(Some(Runtime::Tokio1), tls)
            .context("failed to create Postgres pool")?;

        Ok(Self {
            pool,
            timeout: query_timeout,
        })
    }

    async fn json_query(
        &self,
        sql: &'static str,
        params: &[&(dyn tokio_postgres::types::ToSql + Sync)],
    ) -> Result<Value> {
        let client = timeout(self.timeout, self.pool.get())
            .await
            .context("timed out acquiring a database connection")??;
        let row = timeout(self.timeout, client.query_one(sql, params))
            .await
            .context("database operation timed out")??;
        row.try_get::<_, Value>("result")
            .context("database adapter did not return JSON in column result")
    }
}

#[async_trait]
impl LearningStore for PostgresStore {
    async fn learning_context(&self, recent_sessions: i32) -> Result<Value> {
        self.json_query(
            "SELECT mcp_api.get_learning_context($1::integer) AS result",
            &[&recent_sessions],
        )
        .await
    }

    async fn recent_practice(&self, limit: i32, skill: Option<&str>) -> Result<Value> {
        self.json_query(
            "SELECT mcp_api.get_recent_practice($1::integer, $2::text) AS result",
            &[&limit, &skill],
        )
        .await
    }

    async fn record_practice_json(&self, payload: Value) -> Result<Value> {
        self.json_query(
            "SELECT mcp_api.record_practice_session($1::jsonb) AS result",
            &[&payload],
        )
        .await
    }

    async fn review_queue(&self, limit: i32, category: Option<&str>) -> Result<Value> {
        self.json_query(
            "SELECT mcp_api.get_review_queue($1::integer, $2::text) AS result",
            &[&limit, &category],
        )
        .await
    }

    async fn upsert_weakness_json(&self, patch: Value) -> Result<Value> {
        self.json_query(
            "SELECT mcp_api.upsert_weakness($1::jsonb) AS result",
            &[&patch],
        )
        .await
    }

    async fn data_status(&self) -> Result<Value> {
        self.json_query("SELECT mcp_api.get_data_status() AS result", &[])
            .await
    }

    async fn ping(&self) -> Result<()> {
        let client = timeout(self.timeout, self.pool.get())
            .await
            .context("timed out acquiring a database connection")??;
        timeout(self.timeout, client.simple_query("SELECT 1"))
            .await
            .context("database ping timed out")??;
        Ok(())
    }
}

#[derive(Clone, Default)]
pub struct MockStore;

#[async_trait]
impl LearningStore for MockStore {
    async fn learning_context(&self, recent_sessions: i32) -> Result<Value> {
        Ok(json!({"recent_sessions": recent_sessions}))
    }

    async fn recent_practice(&self, limit: i32, skill: Option<&str>) -> Result<Value> {
        Ok(json!({"items": [], "limit": limit, "skill": skill}))
    }

    async fn record_practice_json(&self, payload: Value) -> Result<Value> {
        Ok(json!({"recorded": true, "payload": payload}))
    }

    async fn review_queue(&self, limit: i32, category: Option<&str>) -> Result<Value> {
        Ok(json!({"items": [], "limit": limit, "category": category}))
    }

    async fn upsert_weakness_json(&self, patch: Value) -> Result<Value> {
        Ok(json!({"updated": true, "patch": patch}))
    }

    async fn data_status(&self) -> Result<Value> {
        Ok(json!({"schema_version": 1, "status": "ready"}))
    }

    async fn ping(&self) -> Result<()> {
        Ok(())
    }
}

pub type SharedStore = Arc<dyn LearningStore>;
