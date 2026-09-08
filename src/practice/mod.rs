pub mod analysis;

use std::{
    env, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, anyhow, bail};
use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path as AxumPath, State, rejection::JsonRejection},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::practice_mvp::{self, AssessmentResponse};

pub const MAX_AUDIO_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_UPLOAD_BODY_BYTES: usize = 12 * 1024 * 1024;
pub const MAX_TASK_CHARS: usize = 2_000;
pub const MAX_AUDIO_DURATION_MS: u64 = 600_000;
pub const SERVER_AUDIO_TTL: Duration = Duration::from_secs(24 * 60 * 60);
pub const ABANDONED_MEDIA_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
const OWNER_KEY: &str = "authenticated-single-learner";

#[derive(Clone)]
pub struct PracticeApiConfig {
    pub media_dir: PathBuf,
    pub model: String,
    pub api_key: Option<String>,
    pub server_audio_cap_bytes: u64,
    pub monthly_spend_cap_usd: f64,
    pub analysis_reservation_usd: f64,
}

impl PracticeApiConfig {
    pub fn from_env(database_path: impl AsRef<Path>) -> Result<Self> {
        let media_dir = env::var_os("PRACTICE_MEDIA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                database_path
                    .as_ref()
                    .parent()
                    .unwrap_or_else(|| Path::new("data"))
                    .join("practice-media")
            });
        fs::create_dir_all(media_dir.join("staging"))
            .with_context(|| format!("create practice media directory {}", media_dir.display()))?;
        fs::create_dir_all(media_dir.join("recordings"))?;
        let model = env::var("GEMINI_MODEL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| practice_mvp::DEFAULT_MODEL.to_owned());
        let api_key = env::var("GEMINI_API_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let server_audio_cap_bytes = env::var("PRACTICE_SERVER_AUDIO_CAP_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(500 * 1024 * 1024);
        let monthly_spend_cap_usd = env::var("PRACTICE_MONTHLY_SPEND_CAP_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(1.0);
        let analysis_reservation_usd = env::var("PRACTICE_ANALYSIS_RESERVATION_USD")
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite() && *value > 0.0)
            .unwrap_or(0.10);
        Ok(Self {
            media_dir,
            model,
            api_key,
            server_audio_cap_bytes,
            monthly_spend_cap_usd,
            analysis_reservation_usd,
        })
    }
}

#[derive(Clone)]
pub struct PracticeApiState {
    pub store: Arc<PracticeJobStore>,
    pub config: Arc<PracticeApiConfig>,
    pub provider: reqwest::Client,
}

pub struct PracticeJobStore {
    connection: Mutex<Connection>,
}

impl PracticeJobStore {
    pub fn new(database_path: impl AsRef<Path>) -> Result<Self> {
        let connection = Connection::open(database_path.as_ref()).with_context(|| {
            format!(
                "open practice job database {}",
                database_path.as_ref().display()
            )
        })?;
        connection.execute_batch("PRAGMA foreign_keys=ON; PRAGMA busy_timeout=10000;")?;
        connection.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS practice_uploads (
                id TEXT PRIMARY KEY NOT NULL,
                owner_key TEXT NOT NULL,
                client_recording_id TEXT NOT NULL,
                idempotency_key TEXT NOT NULL UNIQUE,
                payload_hash TEXT NOT NULL,
                expected_hash TEXT NOT NULL,
                expected_bytes INTEGER NOT NULL,
                mime_type TEXT NOT NULL,
                staging_path TEXT NOT NULL,
                status TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS practice_recordings (
                id TEXT PRIMARY KEY NOT NULL,
                owner_key TEXT NOT NULL,
                upload_id TEXT NOT NULL UNIQUE REFERENCES practice_uploads(id),
                relative_path TEXT NOT NULL,
                hash TEXT NOT NULL,
                bytes INTEGER NOT NULL,
                mime_type TEXT NOT NULL,
                created_at TEXT NOT NULL,
                expires_at TEXT NOT NULL,
                status TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS practice_jobs (
                id TEXT PRIMARY KEY NOT NULL,
                owner_key TEXT NOT NULL,
                recording_id TEXT NOT NULL REFERENCES practice_recordings(id),
                idempotency_key TEXT NOT NULL UNIQUE,
                payload_hash TEXT NOT NULL,
                request_json TEXT NOT NULL,
                status TEXT NOT NULL,
                result_json TEXT,
                error_code TEXT,
                error_message TEXT,
                cancel_requested INTEGER NOT NULL DEFAULT 0,
                lease_until TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                expires_at TEXT NOT NULL
            );
            CREATE TABLE IF NOT EXISTS practice_spend_reservations (
                job_id TEXT PRIMARY KEY NOT NULL REFERENCES practice_jobs(id) ON DELETE CASCADE,
                month_key TEXT NOT NULL,
                amount_usd REAL NOT NULL,
                status TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS practice_jobs_status_idx ON practice_jobs(status, created_at);
            CREATE INDEX IF NOT EXISTS practice_recordings_expiry_idx ON practice_recordings(expires_at);
            ",
        )?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>, PracticeError> {
        self.connection
            .lock()
            .map_err(|_| PracticeError::internal("practice database lock poisoned"))
    }

    fn current_audio_bytes(&self) -> Result<u64, PracticeError> {
        let connection = self.conn()?;
        let bytes = connection.query_row(
            "SELECT COALESCE(SUM(bytes), 0) FROM practice_recordings WHERE status = 'ready' AND mime_type LIKE 'audio/%'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(bytes.max(0) as u64)
    }

    fn find_upload(&self, upload_id: &str) -> Result<UploadRow, PracticeError> {
        let connection = self.conn()?;
        connection
            .query_row(
                "SELECT id, client_recording_id, idempotency_key, payload_hash, expected_hash, expected_bytes, mime_type, staging_path, status, created_at, expires_at FROM practice_uploads WHERE id = ? AND owner_key = ?",
                params![upload_id, OWNER_KEY],
                UploadRow::from_row,
            )
            .optional()?
            .ok_or_else(|| PracticeError::not_found("upload not found"))
    }

    fn find_recording(&self, recording_id: &str) -> Result<RecordingRow, PracticeError> {
        let connection = self.conn()?;
        connection
            .query_row(
                "SELECT id, upload_id, relative_path, hash, bytes, mime_type, created_at, expires_at, status FROM practice_recordings WHERE id = ? AND owner_key = ?",
                params![recording_id, OWNER_KEY],
                RecordingRow::from_row,
            )
            .optional()?
            .ok_or_else(|| PracticeError::not_found("recording not found"))
    }

    fn create_upload(
        &self,
        input: &CreateUploadRequest,
        payload_hash: &str,
        staging_path: &str,
    ) -> Result<UploadRow, PracticeError> {
        let connection = self.conn()?;
        if let Some(existing) = connection
            .query_row(
                "SELECT id, client_recording_id, idempotency_key, payload_hash, expected_hash, expected_bytes, mime_type, staging_path, status, created_at, expires_at FROM practice_uploads WHERE idempotency_key = ? AND owner_key = ?",
                params![input.idempotency_key, OWNER_KEY],
                UploadRow::from_row,
            )
            .optional()?
        {
            if existing.payload_hash != payload_hash {
                return Err(PracticeError::conflict("idempotency key was already used with a different upload"));
            }
            return Ok(existing);
        }
        let id = Uuid::new_v4().to_string();
        let created_at = timestamp_now();
        let expires_at = timestamp_after(ABANDONED_MEDIA_TTL);
        connection.execute(
            "INSERT INTO practice_uploads (id, owner_key, client_recording_id, idempotency_key, payload_hash, expected_hash, expected_bytes, mime_type, staging_path, status, created_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'created', ?, ?)",
            params![
                id,
                OWNER_KEY,
                input.recording_id,
                input.idempotency_key,
                payload_hash,
                input.sha256,
                input.bytes as i64,
                input.mime_type,
                staging_path,
                created_at,
                expires_at,
            ],
        )?;
        Ok(UploadRow {
            id,
            client_recording_id: input.recording_id.clone(),
            idempotency_key: input.idempotency_key.clone(),
            payload_hash: payload_hash.to_owned(),
            expected_hash: input.sha256.clone(),
            expected_bytes: input.bytes,
            mime_type: input.mime_type.clone(),
            staging_path: staging_path.to_owned(),
            status: "created".to_owned(),
            created_at,
            expires_at,
        })
    }

    fn mark_upload_written(&self, upload_id: &str) -> Result<(), PracticeError> {
        let connection = self.conn()?;
        connection.execute(
            "UPDATE practice_uploads SET status = 'uploaded' WHERE id = ? AND owner_key = ?",
            params![upload_id, OWNER_KEY],
        )?;
        Ok(())
    }

    fn finalize_upload(
        &self,
        upload: &UploadRow,
        hash: &str,
        bytes: u64,
        relative_path: &str,
        config: &PracticeApiConfig,
    ) -> Result<RecordingRow, PracticeError> {
        if is_audio_mime(&upload.mime_type) {
            let existing_bytes = self.current_audio_bytes()?;
            if existing_bytes.saturating_add(bytes) > config.server_audio_cap_bytes {
                return Err(PracticeError::too_large(
                    "the server processing audio cap is full; delete a completed recording and retry",
                ));
            }
        }
        let connection = self.conn()?;
        let recording_id = upload.client_recording_id.clone();
        let created_at = timestamp_now();
        let expires_at = timestamp_after(SERVER_AUDIO_TTL);
        connection.execute("BEGIN IMMEDIATE", [])?;
        let result = (|| {
            connection.execute(
                "UPDATE practice_uploads SET status = 'finalized' WHERE id = ? AND owner_key = ?",
                params![upload.id, OWNER_KEY],
            )?;
            connection.execute(
                "INSERT OR REPLACE INTO practice_recordings (id, owner_key, upload_id, relative_path, hash, bytes, mime_type, created_at, expires_at, status) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 'ready')",
                params![recording_id, OWNER_KEY, upload.id, relative_path, hash, bytes as i64, upload.mime_type, created_at, expires_at],
            )?;
            Ok::<(), rusqlite::Error>(())
        })();
        match result {
            Ok(()) => {
                connection.execute("COMMIT", [])?;
                Ok(RecordingRow {
                    id: recording_id,
                    upload_id: upload.id.clone(),
                    relative_path: relative_path.to_owned(),
                    hash: hash.to_owned(),
                    bytes,
                    mime_type: upload.mime_type.clone(),
                    created_at,
                    expires_at,
                    status: "ready".to_owned(),
                })
            }
            Err(error) => {
                let _ = connection.execute("ROLLBACK", []);
                Err(error.into())
            }
        }
    }

    fn create_job(
        &self,
        input: &RequestAnalysisRequest,
        payload_hash: &str,
        config: &PracticeApiConfig,
    ) -> Result<JobRow, PracticeError> {
        let connection = self.conn()?;
        if let Some(existing) = connection
            .query_row(
                "SELECT id, recording_id, idempotency_key, payload_hash, request_json, status, result_json, error_code, error_message, cancel_requested, lease_until, created_at, updated_at, expires_at FROM practice_jobs WHERE idempotency_key = ? AND owner_key = ?",
                params![input.idempotency_key, OWNER_KEY],
                JobRow::from_row,
            )
            .optional()?
        {
            if existing.payload_hash != payload_hash {
                return Err(PracticeError::conflict("idempotency key was already used with a different analysis request"));
            }
            return Ok(existing);
        }
        let id = Uuid::new_v4().to_string();
        let timestamp = timestamp_now();
        let requests_coaching = input
            .requested_stages
            .iter()
            .any(|stage| stage == "gemini" || stage == "coaching");
        let reservation = if requests_coaching {
            input
                .spending_reservation_usd
                .unwrap_or(config.analysis_reservation_usd)
                .max(config.analysis_reservation_usd)
        } else {
            0.0
        };
        if reservation > config.monthly_spend_cap_usd {
            return Err(PracticeError::conflict(
                "the requested analysis reservation exceeds the monthly server cap",
            ));
        }
        let month_key = current_month_key();
        let reserved_this_month = connection.query_row(
            "SELECT COALESCE(SUM(amount_usd), 0) FROM practice_spend_reservations WHERE month_key = ? AND status IN ('reserved', 'settled')",
            params![month_key],
            |row| row.get::<_, f64>(0),
        )?;
        if reserved_this_month + reservation > config.monthly_spend_cap_usd {
            return Err(PracticeError::conflict(
                "the monthly analysis cap has been reached; wait for the next period or increase the server cap",
            ));
        }
        let request_json = serde_json::to_string(input)
            .map_err(|_| PracticeError::invalid("analysis request could not be serialized"))?;
        connection.execute(
            "INSERT INTO practice_jobs (id, owner_key, recording_id, idempotency_key, payload_hash, request_json, status, created_at, updated_at, expires_at) VALUES (?, ?, ?, ?, ?, ?, 'queued', ?, ?, ?)",
            params![id, OWNER_KEY, input.recording_id, input.idempotency_key, payload_hash, request_json, timestamp, timestamp, timestamp_after(ABANDONED_MEDIA_TTL)],
        )?;
        connection.execute(
            "INSERT INTO practice_spend_reservations (job_id, month_key, amount_usd, status) VALUES (?, ?, ?, 'reserved')",
            params![id, current_month_key(), reservation],
        )?;
        Ok(JobRow {
            id,
            recording_id: input.recording_id.clone(),
            idempotency_key: input.idempotency_key.clone(),
            payload_hash: payload_hash.to_owned(),
            request_json,
            status: "queued".to_owned(),
            result_json: None,
            error_code: None,
            error_message: None,
            cancel_requested: false,
            lease_until: None,
            created_at: timestamp.clone(),
            updated_at: timestamp,
            expires_at: timestamp_after(ABANDONED_MEDIA_TTL),
        })
    }

    fn find_job(&self, job_id: &str) -> Result<JobRow, PracticeError> {
        let connection = self.conn()?;
        connection
            .query_row(
                "SELECT id, recording_id, idempotency_key, payload_hash, request_json, status, result_json, error_code, error_message, cancel_requested, lease_until, created_at, updated_at, expires_at FROM practice_jobs WHERE id = ? AND owner_key = ?",
                params![job_id, OWNER_KEY],
                JobRow::from_row,
            )
            .optional()?
            .ok_or_else(|| PracticeError::not_found("analysis job not found"))
    }

    fn claim_job(&self) -> Result<Option<JobWork>, PracticeError> {
        let connection = self.conn()?;
        connection.execute(
            "UPDATE practice_jobs SET status = 'queued', lease_until = NULL, updated_at = ? WHERE owner_key = ? AND status = 'running' AND cancel_requested = 0 AND lease_until < ?",
            params![timestamp_now(), OWNER_KEY, timestamp_now()],
        )?;
        let job = connection
            .query_row(
                "SELECT id, recording_id, request_json FROM practice_jobs WHERE owner_key = ? AND status = 'queued' AND cancel_requested = 0 ORDER BY created_at ASC LIMIT 1",
                params![OWNER_KEY],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?)),
            )
            .optional()?;
        let Some((id, recording_id, request_json)) = job else {
            return Ok(None);
        };
        let recording = connection
            .query_row(
                "SELECT relative_path, mime_type FROM practice_recordings WHERE id = ? AND owner_key = ? AND status = 'ready'",
                params![recording_id, OWNER_KEY],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?;
        let Some((relative_path, mime_type)) = recording else {
            connection.execute("UPDATE practice_jobs SET status = 'failed', error_code = 'recording_missing', error_message = 'recording is no longer available', updated_at = ? WHERE id = ?", params![timestamp_now(), id])?;
            connection.execute(
                "UPDATE practice_spend_reservations SET status = 'released' WHERE job_id = ?",
                params![id],
            )?;
            return Ok(None);
        };
        let lease_until = timestamp_after(Duration::from_secs(10 * 60));
        connection.execute(
            "UPDATE practice_jobs SET status = 'running', lease_until = ?, updated_at = ? WHERE id = ? AND status = 'queued'",
            params![lease_until, timestamp_now(), id],
        )?;
        Ok(Some(JobWork {
            id,
            recording_id,
            relative_path,
            mime_type,
            request_json,
        }))
    }

    fn finish_job(
        &self,
        job_id: &str,
        status: &str,
        result: Option<&Value>,
        error: Option<(&str, &str)>,
    ) -> Result<(), PracticeError> {
        let connection = self.conn()?;
        let result_json = result
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| PracticeError::internal("job result could not be serialized"))?;
        connection.execute(
            "UPDATE practice_jobs SET status = CASE WHEN cancel_requested = 1 THEN 'cancelled' ELSE ? END, result_json = ?, error_code = ?, error_message = ?, lease_until = NULL, updated_at = ? WHERE id = ? AND owner_key = ?",
            params![status, result_json, error.map(|value| value.0), error.map(|value| value.1), timestamp_now(), job_id, OWNER_KEY],
        )?;
        connection.execute(
            "UPDATE practice_spend_reservations SET status = CASE WHEN EXISTS (SELECT 1 FROM practice_jobs WHERE id = ? AND status = 'completed') THEN 'settled' ELSE 'released' END WHERE job_id = ?",
            params![job_id, job_id],
        )?;
        Ok(())
    }

    fn cancellation_requested(&self, job_id: &str) -> Result<bool, PracticeError> {
        let connection = self.conn()?;
        let cancelled = connection.query_row(
            "SELECT cancel_requested FROM practice_jobs WHERE id = ? AND owner_key = ?",
            params![job_id, OWNER_KEY],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(cancelled != 0)
    }

    fn cancel_job(&self, job_id: &str) -> Result<JobRow, PracticeError> {
        let connection = self.conn()?;
        connection.execute(
            "UPDATE practice_jobs SET cancel_requested = 1, status = CASE WHEN status IN ('queued', 'running') THEN 'cancelled' ELSE status END, updated_at = ? WHERE id = ? AND owner_key = ?",
            params![timestamp_now(), job_id, OWNER_KEY],
        )?;
        connection.execute(
            "UPDATE practice_spend_reservations SET status = 'released' WHERE job_id = ? AND status = 'reserved'",
            params![job_id],
        )?;
        drop(connection);
        self.find_job(job_id)
    }

    fn delete_recording(&self, recording_id: &str) -> Result<RecordingRow, PracticeError> {
        let connection = self.conn()?;
        let recording = connection
            .query_row(
                "SELECT id, upload_id, relative_path, hash, bytes, mime_type, created_at, expires_at, status FROM practice_recordings WHERE id = ? AND owner_key = ?",
                params![recording_id, OWNER_KEY],
                RecordingRow::from_row,
            )
            .optional()?
            .ok_or_else(|| PracticeError::not_found("recording not found"))?;
        connection.execute(
            "UPDATE practice_recordings SET status = 'deleted' WHERE id = ? AND owner_key = ?",
            params![recording_id, OWNER_KEY],
        )?;
        connection.execute("UPDATE practice_jobs SET cancel_requested = 1, status = CASE WHEN status IN ('queued', 'running') THEN 'cancelled' ELSE status END, updated_at = ? WHERE recording_id = ? AND owner_key = ?", params![timestamp_now(), recording_id, OWNER_KEY])?;
        connection.execute(
            "UPDATE practice_spend_reservations SET status = 'released' WHERE job_id IN (SELECT id FROM practice_jobs WHERE recording_id = ?) AND status = 'reserved'",
            params![recording_id],
        )?;
        Ok(recording)
    }

    fn cleanup_expired(&self, media_dir: &Path) -> Result<u64, PracticeError> {
        let connection = self.conn()?;
        let now = timestamp_now();
        let paths = connection
            .prepare("SELECT relative_path FROM practice_recordings WHERE expires_at < ? AND status = 'ready'")?
            .query_map(params![now.clone()], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut removed = 0;
        for relative_path in paths {
            let path =
                safe_media_path(media_dir, &relative_path).map_err(PracticeError::from_anyhow)?;
            let _ = fs::remove_file(path);
            removed += connection.execute("UPDATE practice_recordings SET status = 'expired' WHERE relative_path = ? AND status = 'ready'", params![relative_path])? as u64;
        }
        connection.execute(
            "DELETE FROM practice_uploads WHERE expires_at < ? AND status != 'finalized'",
            params![now.clone()],
        )?;
        connection.execute(
            "UPDATE practice_jobs SET status = 'failed', error_code = 'expired', error_message = 'analysis job expired before completion', lease_until = NULL, updated_at = ? WHERE expires_at < ? AND status IN ('queued', 'running')",
            params![timestamp_now(), now.clone()],
        )?;
        connection.execute(
            "UPDATE practice_spend_reservations SET status = 'released' WHERE job_id IN (SELECT id FROM practice_jobs WHERE expires_at < ? AND status = 'failed') AND status = 'reserved'",
            params![now.clone()],
        )?;
        connection.execute("DELETE FROM practice_jobs WHERE expires_at < ? AND status IN ('completed', 'failed', 'cancelled')", params![now])?;
        Ok(removed)
    }
}

pub fn router(database_path: impl AsRef<Path>) -> Result<Router> {
    let config = Arc::new(PracticeApiConfig::from_env(database_path.as_ref())?);
    let store = Arc::new(PracticeJobStore::new(database_path)?);
    let provider = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .context("build practice provider client")?;
    let state = PracticeApiState {
        store,
        config,
        provider,
    };
    Ok(Router::new()
        .route("/api/practice/v1/uploads", post(create_upload))
        .route(
            "/api/practice/v1/uploads/{upload_id}/content",
            put(upload_content),
        )
        .route(
            "/api/practice/v1/uploads/{upload_id}/finalize",
            post(finalize_upload),
        )
        .route("/api/practice/v1/analyses", post(request_analysis))
        .route("/api/practice/v1/jobs/{job_id}", get(read_job))
        .route("/api/practice/v1/jobs/{job_id}/cancel", post(cancel_job))
        .route(
            "/api/practice/v1/recordings/{recording_id}",
            delete(delete_recording),
        )
        .layer(DefaultBodyLimit::max(MAX_UPLOAD_BODY_BYTES))
        .with_state(state))
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateUploadRequest {
    pub recording_id: String,
    pub sha256: String,
    pub bytes: u64,
    pub mime_type: String,
    #[serde(default)]
    pub kind: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Serialize)]
pub struct CreateUploadResponse {
    pub upload_id: String,
    pub recording_id: String,
    pub max_bytes: u64,
    pub max_duration_ms: u64,
    pub expires_at: String,
    pub status: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RequestAnalysisRequest {
    pub recording_id: String,
    pub idempotency_key: String,
    pub task: String,
    pub duration_ms: u64,
    pub requested_stages: Vec<String>,
    pub spending_reservation_usd: Option<f64>,
    pub image_upload_id: Option<String>,
    pub image_hash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct JobResponse {
    pub job_id: String,
    pub recording_id: String,
    pub status: String,
    pub result: Option<Value>,
    pub error: Option<JobErrorResponse>,
    pub expires_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize)]
pub struct JobErrorResponse {
    pub code: String,
    pub message: String,
}

async fn create_upload(
    State(state): State<PracticeApiState>,
    input: Result<Json<CreateUploadRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<CreateUploadResponse>), PracticeError> {
    let Json(input) = input.map_err(|_| {
        PracticeError::invalid("request JSON is invalid or contains an unknown field")
    })?;
    validate_upload_request(&input)?;
    let payload_hash = hash_json(&input)?;
    let staging_path = state
        .config
        .media_dir
        .join("staging")
        .join(format!("{}.upload", Uuid::new_v4()));
    let staging_path_text = staging_path.to_string_lossy().into_owned();
    let upload = state
        .store
        .create_upload(&input, &payload_hash, &staging_path_text)?;
    Ok((
        StatusCode::CREATED,
        Json(CreateUploadResponse {
            upload_id: upload.id,
            recording_id: upload.client_recording_id,
            max_bytes: max_upload_bytes(&upload.mime_type),
            max_duration_ms: if is_audio_mime(&upload.mime_type) {
                MAX_AUDIO_DURATION_MS
            } else {
                0
            },
            expires_at: upload.expires_at,
            status: upload.status,
        }),
    ))
}

async fn upload_content(
    State(state): State<PracticeApiState>,
    AxumPath(upload_id): AxumPath<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>, PracticeError> {
    let upload = state.store.find_upload(&upload_id)?;
    if upload.status == "finalized" {
        return Ok(Json(
            json!({ "status": "already_uploaded", "upload_id": upload.id }),
        ));
    }
    if body.len() as u64 != upload.expected_bytes
        || body.len() as u64 > MAX_UPLOAD_BODY_BYTES as u64
    {
        return Err(PracticeError::too_large(
            "upload size does not match the reserved byte count",
        ));
    }
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !content_type.is_empty() && normalize_mime(content_type) != normalize_mime(&upload.mime_type)
    {
        return Err(PracticeError::unsupported(
            "the content MIME type does not match the upload declaration",
        ));
    }
    fs::write(&upload.staging_path, &body)
        .map_err(|error| PracticeError::internal(format!("write upload: {error}")))?;
    state.store.mark_upload_written(&upload.id)?;
    Ok(Json(
        json!({ "status": "uploaded", "upload_id": upload.id, "bytes": body.len() }),
    ))
}

async fn finalize_upload(
    State(state): State<PracticeApiState>,
    AxumPath(upload_id): AxumPath<String>,
) -> Result<Json<Value>, PracticeError> {
    let upload = state.store.find_upload(&upload_id)?;
    if upload.status == "finalized" {
        let recording = state.store.find_recording(&upload.client_recording_id)?;
        return Ok(Json(json!({
            "status": "ready",
            "recording_id": recording.id,
            "hash": recording.hash,
            "bytes": recording.bytes,
            "mime_type": recording.mime_type,
            "expires_at": recording.expires_at,
        })));
    }
    let staging = PathBuf::from(&upload.staging_path);
    let bytes = fs::read(&staging)
        .map_err(|_| PracticeError::invalid("upload content has not been provided"))?;
    if bytes.len() as u64 != upload.expected_bytes {
        return Err(PracticeError::too_large(
            "upload size does not match the declared size",
        ));
    }
    let hash = sha256_hex(&bytes);
    if !constant_time_string_eq(&hash, &upload.expected_hash) {
        return Err(PracticeError::invalid(
            "upload hash does not match the declared hash",
        ));
    }
    let relative_path = format!(
        "recordings/{}{}",
        upload.client_recording_id,
        extension_for_mime(&upload.mime_type)
    );
    let destination = safe_media_path(&state.config.media_dir, &relative_path)
        .map_err(PracticeError::from_anyhow)?;
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| PracticeError::internal(format!("create media directory: {error}")))?;
    }
    fs::rename(&staging, &destination)
        .or_else(|_| {
            fs::copy(&staging, &destination)
                .map(|_| ())
                .and_then(|_| fs::remove_file(&staging))
        })
        .map_err(|error| PracticeError::internal(format!("finalize upload: {error}")))?;
    let recording = state.store.finalize_upload(
        &upload,
        &hash,
        bytes.len() as u64,
        &relative_path,
        &state.config,
    )?;
    Ok(Json(json!({
        "status": "ready",
        "recording_id": recording.id,
        "hash": recording.hash,
        "bytes": recording.bytes,
        "mime_type": recording.mime_type,
        "expires_at": recording.expires_at,
    })))
}

async fn request_analysis(
    State(state): State<PracticeApiState>,
    input: Result<Json<RequestAnalysisRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<JobResponse>), PracticeError> {
    let Json(input) = input.map_err(|_| {
        PracticeError::invalid("request JSON is invalid or contains an unknown field")
    })?;
    validate_analysis_request(&input)?;
    let recording = state.store.find_recording(&input.recording_id)?;
    if recording.status != "ready" {
        return Err(PracticeError::conflict(
            "recording is no longer available for analysis",
        ));
    }
    if let Some(image_upload_id) = input.image_upload_id.as_deref() {
        let image = state.store.find_upload(image_upload_id)?;
        if !is_image_mime(&image.mime_type) || image.status != "finalized" {
            return Err(PracticeError::unsupported(
                "the image upload is not finalized or is not a supported image",
            ));
        }
        if input.image_hash.as_deref() != Some(image.expected_hash.as_str()) {
            return Err(PracticeError::invalid(
                "image hash does not match the finalized image",
            ));
        }
        let image_recording = state.store.find_recording(&image.client_recording_id)?;
        if image_recording.status != "ready" {
            return Err(PracticeError::conflict(
                "the finalized image is no longer available",
            ));
        }
    } else if input.image_hash.is_some() {
        return Err(PracticeError::invalid(
            "image_hash requires image_upload_id",
        ));
    }
    let payload_hash = hash_json(&input)?;
    let job = state
        .store
        .create_job(&input, &payload_hash, &state.config)?;
    Ok((StatusCode::ACCEPTED, Json(job_response(job)?)))
}

async fn read_job(
    State(state): State<PracticeApiState>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobResponse>, PracticeError> {
    Ok(Json(job_response(state.store.find_job(&job_id)?)?))
}

async fn cancel_job(
    State(state): State<PracticeApiState>,
    AxumPath(job_id): AxumPath<String>,
) -> Result<Json<JobResponse>, PracticeError> {
    Ok(Json(job_response(state.store.cancel_job(&job_id)?)?))
}

async fn delete_recording(
    State(state): State<PracticeApiState>,
    AxumPath(recording_id): AxumPath<String>,
) -> Result<StatusCode, PracticeError> {
    let recording = match state.store.delete_recording(&recording_id) {
        Ok(recording) => recording,
        Err(PracticeError::NotFound(_)) => return Ok(StatusCode::NO_CONTENT),
        Err(error) => return Err(error),
    };
    let path = safe_media_path(&state.config.media_dir, &recording.relative_path)
        .map_err(PracticeError::from_anyhow)?;
    let _ = fs::remove_file(path);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn worker_once(state: &PracticeApiState) -> Result<bool> {
    let Some(work) = state
        .store
        .claim_job()
        .map_err(|error| anyhow!(error.to_string()))?
    else {
        let _ = state.store.cleanup_expired(&state.config.media_dir);
        return Ok(false);
    };
    let result = if state.store.cancellation_requested(&work.id)? {
        Ok(json!({ "cancelled": true }))
    } else {
        process_job(state, &work).await
    };
    match result {
        Ok(value) => state
            .store
            .finish_job(&work.id, "completed", Some(&value), None)
            .map_err(|error| anyhow!(error.to_string()))?,
        Err(error) => {
            let (code, message) = sanitize_worker_error(&error);
            state
                .store
                .finish_job(&work.id, "failed", None, Some((code, message)))
                .map_err(|store_error| anyhow!(store_error.to_string()))?;
        }
    }
    Ok(true)
}

async fn process_job(state: &PracticeApiState, work: &JobWork) -> Result<Value> {
    let request: RequestAnalysisRequest = serde_json::from_str(&work.request_json)?;
    let recording_path = safe_media_path(&state.config.media_dir, &work.relative_path)?;
    let audio = fs::read(&recording_path).with_context(|| "read staged recording")?;
    if audio.len() as u64 > MAX_AUDIO_BYTES {
        bail!("audio exceeds worker limit")
    }

    let mut limitations = Vec::new();
    let metrics = if let Some((samples, sample_rate)) = analysis::decode_bounded_audio(&audio) {
        serde_json::to_value(analysis::analyze_pcm16le(&samples, sample_rate))?
    } else {
        limitations.push("The retained audio could not be decoded into the bounded analysis format; delivery metrics are unavailable.".to_owned());
        Value::Null
    };

    let mut response = json!({
        "recording_id": work.recording_id,
        "processor_version": analysis::PROCESSOR_VERSION,
        "model_version": analysis::VAD_MODEL_VERSION,
        "config": {
            "sample_rate_hz": analysis::SAMPLE_RATE_HZ,
            "probability_threshold": analysis::PROBABILITY_THRESHOLD,
            "minimum_speech_ms": analysis::MINIMUM_SPEECH_MS,
            "minimum_silence_ms": analysis::MINIMUM_SILENCE_MS,
            "internal_pause_ms": analysis::INTERNAL_PAUSE_MS,
            "long_pause_ms": analysis::LONG_PAUSE_MS,
        },
        "metrics": metrics,
        "limitations": limitations,
    });

    if request
        .requested_stages
        .iter()
        .any(|stage| stage == "gemini" || stage == "coaching")
    {
        let Some(api_key) = state.config.api_key.as_deref() else {
            limitations.push(
                "Gemini coaching was requested but the server has no configured provider key."
                    .to_owned(),
            );
            response["limitations"] = serde_json::to_value(limitations)?;
            return Ok(response);
        };
        let mut visual_coverage_unavailable = false;
        let image_bytes = if let Some(image_upload_id) = request.image_upload_id.as_deref() {
            let image_upload = state.store.find_upload(image_upload_id)?;
            let image_recording = state
                .store
                .find_recording(&image_upload.client_recording_id)?;
            let image_path =
                safe_media_path(&state.config.media_dir, &image_recording.relative_path)?;
            match fs::read(image_path) {
                Ok(bytes) => match validate_task_image(&image_recording.mime_type, &bytes) {
                    Ok(mime_type) => Some((bytes, mime_type)),
                    Err(_) => {
                        visual_coverage_unavailable = true;
                        limitations.push(
                            "Visual coverage is unavailable because the selected image could not be decoded safely."
                                .to_owned(),
                        );
                        None
                    }
                },
                Err(_) => {
                    visual_coverage_unavailable = true;
                    limitations.push(
                        "Visual coverage is unavailable because the selected image is no longer available."
                            .to_owned(),
                    );
                    None
                }
            }
        } else {
            None
        };
        let image = image_bytes
            .as_ref()
            .map(|(bytes, mime_type)| (bytes.as_slice(), mime_type.as_str()));
        let mut feedback = practice_mvp::gemini::request_feedback_with_image(
            &state.provider,
            api_key,
            &state.config.model,
            &audio,
            practice_mvp::gemini::provider_mime_type(&work.mime_type, &audio)
                .map_err(|_| anyhow!("unsupported audio container"))?,
            &request.task,
            request.duration_ms,
            image,
            "practice-worker",
        )
        .await
        .map_err(|_| anyhow!("provider analysis failed"))?;
        if visual_coverage_unavailable {
            feedback.feedback.limitations.push(
                "Visual coverage is unavailable; this coaching result uses the audio and task only."
                    .to_owned(),
            );
        }
        response["coaching"] = serde_json::to_value(AssessmentResponse {
            feedback: feedback.feedback,
            model: feedback.model,
            elapsed_ms: 0,
            usage: feedback.usage,
        })?;
    }
    response["limitations"] = serde_json::to_value(&limitations)?;
    Ok(response)
}

fn validate_task_image(mime_type: &str, bytes: &[u8]) -> Result<String> {
    let provider_mime = practice_mvp::gemini::provider_image_mime_type(mime_type, bytes)
        .map_err(|_| anyhow!("unsupported task image"))?;
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("image format could not be identified")?;
    let (width, height) = reader
        .into_dimensions()
        .context("image dimensions unavailable")?;
    if width == 0
        || height == 0
        || width > 4_096
        || height > 4_096
        || u64::from(width) * u64::from(height) > 16_000_000
    {
        bail!("task image dimensions exceed the safe processing limit");
    }
    image::load_from_memory(bytes).context("task image could not be decoded")?;
    Ok(provider_mime.to_owned())
}

fn job_response(job: JobRow) -> Result<JobResponse, PracticeError> {
    let result = job
        .result_json
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|_| PracticeError::internal("stored job result is invalid"))?;
    Ok(JobResponse {
        job_id: job.id,
        recording_id: job.recording_id,
        status: job.status,
        result,
        error: job
            .error_code
            .zip(job.error_message)
            .map(|(code, message)| JobErrorResponse { code, message }),
        expires_at: job.expires_at,
        updated_at: job.updated_at,
    })
}

fn validate_upload_request(input: &CreateUploadRequest) -> Result<(), PracticeError> {
    if input.recording_id.trim().is_empty()
        || input.recording_id.chars().count() > 128
        || !input.recording_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(PracticeError::invalid(
            "recording_id is required and bounded",
        ));
    }
    if input.idempotency_key.trim().is_empty() || input.idempotency_key.chars().count() > 160 {
        return Err(PracticeError::invalid(
            "idempotency_key is required and bounded",
        ));
    }
    if input.sha256.len() != 64
        || !input
            .sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(PracticeError::invalid(
            "sha256 must be a 64-character hexadecimal digest",
        ));
    }
    let is_image = input
        .kind
        .as_deref()
        .map(|kind| kind == "image")
        .unwrap_or_else(|| is_image_mime(&input.mime_type));
    let is_audio = input
        .kind
        .as_deref()
        .map(|kind| kind == "audio")
        .unwrap_or_else(|| is_audio_mime(&input.mime_type));
    if !is_image && !is_audio {
        return Err(PracticeError::invalid(
            "kind must be audio or image and match the MIME type",
        ));
    }
    if (is_image && !is_image_mime(&input.mime_type))
        || (is_audio && !is_audio_mime(&input.mime_type))
    {
        return Err(PracticeError::unsupported(
            "the declared kind does not match the MIME type",
        ));
    }
    if input.bytes == 0 || input.bytes > max_upload_bytes(&input.mime_type) {
        return Err(PracticeError::too_large(
            "the upload is empty or exceeds its media type limit",
        ));
    }
    Ok(())
}

fn validate_analysis_request(input: &RequestAnalysisRequest) -> Result<(), PracticeError> {
    if input.recording_id.trim().is_empty() || input.idempotency_key.trim().is_empty() {
        return Err(PracticeError::invalid(
            "recording_id and idempotency_key are required",
        ));
    }
    if input.task.trim().is_empty() || input.task.chars().count() > MAX_TASK_CHARS {
        return Err(PracticeError::invalid("task is required and bounded"));
    }
    if input.duration_ms == 0 || input.duration_ms > MAX_AUDIO_DURATION_MS {
        return Err(PracticeError::invalid(
            "duration_ms must be between 1 ms and 10 minutes",
        ));
    }
    if input.requested_stages.is_empty() || input.requested_stages.len() > 4 {
        return Err(PracticeError::invalid(
            "requested_stages must contain one to four stages",
        ));
    }
    if input
        .spending_reservation_usd
        .is_some_and(|value| !value.is_finite() || value < 0.0)
    {
        return Err(PracticeError::invalid(
            "spending_reservation_usd must be finite and nonnegative",
        ));
    }
    Ok(())
}

fn is_audio_mime(value: &str) -> bool {
    matches!(
        normalize_mime(value).as_str(),
        "audio/m4a" | "audio/mp4" | "audio/webm" | "audio/ogg" | "audio/opus"
    )
}

fn is_image_mime(value: &str) -> bool {
    matches!(
        normalize_mime(value).as_str(),
        "image/jpeg" | "image/png" | "image/webp"
    )
}

fn max_upload_bytes(mime_type: &str) -> u64 {
    if is_image_mime(mime_type) {
        MAX_IMAGE_BYTES
    } else {
        MAX_AUDIO_BYTES
    }
}

fn normalize_mime(value: &str) -> String {
    value
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
}

fn extension_for_mime(value: &str) -> &'static str {
    match normalize_mime(value).as_str() {
        "audio/m4a" | "audio/mp4" => ".m4a",
        "audio/ogg" | "audio/opus" => ".ogg",
        "image/png" => ".png",
        "image/webp" => ".webp",
        _ => ".webm",
    }
}

fn safe_media_path(media_dir: &Path, relative_path: &str) -> Result<PathBuf> {
    let path = Path::new(relative_path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        bail!("invalid managed media path")
    }
    Ok(media_dir.join(path))
}

fn hash_json<T: Serialize>(value: &T) -> Result<String, PracticeError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|_| PracticeError::invalid("request could not be serialized"))?;
    Ok(sha256_hex(&bytes))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn constant_time_string_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

fn timestamp_now() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn current_month_key() -> String {
    chrono::Utc::now().format("%Y-%m").to_string()
}

fn timestamp_after(duration: Duration) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let seconds = now.as_secs().saturating_add(duration.as_secs());
    chrono::DateTime::<chrono::Utc>::from_timestamp(seconds as i64, 0)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

fn sanitize_worker_error(error: &anyhow::Error) -> (&'static str, &'static str) {
    let _ = error;
    (
        "processing_failed",
        "The analysis worker could not publish a result.",
    )
}

#[derive(Debug)]
#[allow(dead_code)]
struct UploadRow {
    id: String,
    client_recording_id: String,
    idempotency_key: String,
    payload_hash: String,
    expected_hash: String,
    expected_bytes: u64,
    mime_type: String,
    staging_path: String,
    status: String,
    created_at: String,
    expires_at: String,
}

impl UploadRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            client_recording_id: row.get(1)?,
            idempotency_key: row.get(2)?,
            payload_hash: row.get(3)?,
            expected_hash: row.get(4)?,
            expected_bytes: row.get::<_, i64>(5)?.max(0) as u64,
            mime_type: row.get(6)?,
            staging_path: row.get(7)?,
            status: row.get(8)?,
            created_at: row.get(9)?,
            expires_at: row.get(10)?,
        })
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct RecordingRow {
    id: String,
    upload_id: String,
    relative_path: String,
    hash: String,
    bytes: u64,
    mime_type: String,
    created_at: String,
    expires_at: String,
    status: String,
}

impl RecordingRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            upload_id: row.get(1)?,
            relative_path: row.get(2)?,
            hash: row.get(3)?,
            bytes: row.get::<_, i64>(4)?.max(0) as u64,
            mime_type: row.get(5)?,
            created_at: row.get(6)?,
            expires_at: row.get(7)?,
            status: row.get(8)?,
        })
    }
}

#[derive(Debug)]
#[allow(dead_code)]
struct JobRow {
    id: String,
    recording_id: String,
    idempotency_key: String,
    payload_hash: String,
    request_json: String,
    status: String,
    result_json: Option<String>,
    error_code: Option<String>,
    error_message: Option<String>,
    cancel_requested: bool,
    lease_until: Option<String>,
    created_at: String,
    updated_at: String,
    expires_at: String,
}

impl JobRow {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            recording_id: row.get(1)?,
            idempotency_key: row.get(2)?,
            payload_hash: row.get(3)?,
            request_json: row.get(4)?,
            status: row.get(5)?,
            result_json: row.get(6)?,
            error_code: row.get(7)?,
            error_message: row.get(8)?,
            cancel_requested: row.get::<_, i64>(9)? != 0,
            lease_until: row.get(10)?,
            created_at: row.get(11)?,
            updated_at: row.get(12)?,
            expires_at: row.get(13)?,
        })
    }
}

#[derive(Debug)]
struct JobWork {
    id: String,
    recording_id: String,
    relative_path: String,
    mime_type: String,
    request_json: String,
}

#[derive(Debug)]
pub enum PracticeError {
    Invalid(String),
    TooLarge(String),
    Unsupported(String),
    NotFound(String),
    Conflict(String),
    Internal(String),
}

impl PracticeError {
    fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }
    fn too_large(message: impl Into<String>) -> Self {
        Self::TooLarge(message.into())
    }
    fn unsupported(message: impl Into<String>) -> Self {
        Self::Unsupported(message.into())
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::NotFound(message.into())
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self::Conflict(message.into())
    }
    fn internal(message: impl Into<String>) -> Self {
        Self::Internal(message.into())
    }
    fn from_anyhow(error: anyhow::Error) -> Self {
        Self::Internal(error.to_string())
    }
}

impl From<rusqlite::Error> for PracticeError {
    fn from(error: rusqlite::Error) -> Self {
        Self::internal(format!("database operation failed: {error}"))
    }
}

impl IntoResponse for PracticeError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::Invalid(message) => (StatusCode::BAD_REQUEST, "invalid_request", message),
            Self::TooLarge(message) => (StatusCode::PAYLOAD_TOO_LARGE, "too_large", message),
            Self::Unsupported(message) => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_media",
                message,
            ),
            Self::NotFound(message) => (StatusCode::NOT_FOUND, "not_found", message),
            Self::Conflict(message) => (StatusCode::CONFLICT, "conflict", message),
            Self::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "The practice service could not complete the request.".to_owned(),
            ),
        };
        (
            status,
            Json(json!({ "error": { "code": code, "message": message } })),
        )
            .into_response()
    }
}

impl std::fmt::Display for PracticeError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(message)
            | Self::TooLarge(message)
            | Self::Unsupported(message)
            | Self::NotFound(message)
            | Self::Conflict(message)
            | Self::Internal(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for PracticeError {}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    fn test_directory() -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("aprendiendo-practice-test-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn rejects_unsupported_mime_and_unbounded_payloads() {
        let input = CreateUploadRequest {
            recording_id: "recording".to_owned(),
            sha256: "0".repeat(64),
            bytes: MAX_AUDIO_BYTES + 1,
            mime_type: "audio/wav".to_owned(),
            kind: None,
            idempotency_key: "one".to_owned(),
        };
        assert!(validate_upload_request(&input).is_err());
        assert!(!is_audio_mime("audio/wav"));
        assert!(is_image_mime("image/jpeg"));
    }

    #[test]
    fn managed_paths_cannot_escape_media_directory() {
        assert!(safe_media_path(Path::new("/tmp/media"), "../secret").is_err());
        assert!(safe_media_path(Path::new("/tmp/media"), "recordings/a.m4a").is_ok());
    }

    #[tokio::test]
    async fn upload_finalize_and_job_creation_are_idempotent() {
        let directory = test_directory();
        let database_path = directory.join("practice.sqlite3");
        let config = PracticeApiConfig {
            media_dir: directory.join("media"),
            model: practice_mvp::DEFAULT_MODEL.to_owned(),
            api_key: None,
            server_audio_cap_bytes: 20 * 1024 * 1024,
            monthly_spend_cap_usd: 1.0,
            analysis_reservation_usd: 0.10,
        };
        fs::create_dir_all(config.media_dir.join("staging")).unwrap();
        fs::create_dir_all(config.media_dir.join("recordings")).unwrap();
        let store = Arc::new(PracticeJobStore::new(&database_path).unwrap());
        let bytes = b"webm-ish";
        let input = CreateUploadRequest {
            recording_id: "recording-1".to_owned(),
            sha256: sha256_hex(bytes),
            bytes: bytes.len() as u64,
            mime_type: "audio/webm".to_owned(),
            kind: None,
            idempotency_key: "upload-1".to_owned(),
        };
        let staging = config.media_dir.join("staging/upload");
        let upload = store
            .create_upload(
                &input,
                &hash_json(&input).unwrap(),
                staging.to_str().unwrap(),
            )
            .unwrap();
        fs::write(&staging, bytes).unwrap();
        store.mark_upload_written(&upload.id).unwrap();
        let destination = "recordings/recording-1.webm";
        fs::write(config.media_dir.join(destination), bytes).unwrap();
        let recording = store
            .finalize_upload(
                &upload,
                &sha256_hex(bytes),
                bytes.len() as u64,
                destination,
                &config,
            )
            .unwrap();
        let request = RequestAnalysisRequest {
            recording_id: recording.id,
            idempotency_key: "job-1".to_owned(),
            task: "Habla.".to_owned(),
            duration_ms: 1_000,
            requested_stages: vec!["measurements".to_owned()],
            spending_reservation_usd: Some(0.0),
            image_upload_id: None,
            image_hash: None,
        };
        let job = store
            .create_job(&request, &hash_json(&request).unwrap(), &config)
            .unwrap();
        let replay = store
            .create_job(&request, &hash_json(&request).unwrap(), &config)
            .unwrap();
        assert_eq!(job.id, replay.id);
        assert_eq!(job.status, "queued");
    }

    #[tokio::test]
    async fn malformed_json_is_bounded_by_the_route_contract() {
        let directory = test_directory();
        let router = router(directory.join("practice.sqlite3")).unwrap();
        let response = router
            .oneshot(
                axum::http::Request::post("/api/practice/v1/uploads")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = axum::body::to_bytes(response.into_body(), 1_000_000)
            .await
            .unwrap();
        assert!(String::from_utf8_lossy(&body).contains("invalid_request"));
    }
}
