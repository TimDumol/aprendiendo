pub mod gemini;

use std::{
    env,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Extension, Multipart, State, multipart::MultipartRejection},
    http::{HeaderValue, Method, Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::sync::Semaphore;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

pub const DEFAULT_MODEL: &str = "gemini-3.8-flash";
pub const DEFAULT_WEB_ORIGIN: &str = "http://localhost:8081";
pub const LISTENER_ADDRESS: &str = "127.0.0.1:8082";
pub const MULTIPART_BODY_LIMIT: usize = 12 * 1024 * 1024;
pub const MAX_AUDIO_BYTES: usize = 10 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 5 * 1024 * 1024;
pub const MAX_TASK_CHARS: usize = 2_000;
pub const MIN_DURATION_MS: u64 = 10_000;
pub const MAX_DURATION_MS: u64 = 300_000;
const PROVIDER_TIMEOUT: Duration = Duration::from_secs(120);
static NEXT_REQUEST_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
struct RequestId(String);

pub fn listener_address() -> String {
    env::var("MVP_BIND_ADDR")
        .ok()
        .filter(|address| !address.trim().is_empty())
        .unwrap_or_else(|| LISTENER_ADDRESS.to_owned())
}

#[derive(Clone)]
pub struct MvpConfig {
    model: String,
    web_origin: String,
    api_key: Option<String>,
}

impl MvpConfig {
    pub fn from_env() -> Result<Self> {
        let model = env::var("GEMINI_MODEL")
            .ok()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| DEFAULT_MODEL.to_owned());
        let web_origin = normalize_web_origin(
            env::var("MVP_WEB_ORIGIN").unwrap_or_else(|_| DEFAULT_WEB_ORIGIN.to_owned()),
        )?;
        let api_key = env::var("GEMINI_API_KEY")
            .ok()
            .filter(|key| !key.trim().is_empty());

        Ok(Self {
            model,
            web_origin,
            api_key,
        })
    }

    pub fn for_tests(api_key: Option<&str>) -> Self {
        Self {
            model: DEFAULT_MODEL.to_owned(),
            web_origin: DEFAULT_WEB_ORIGIN.to_owned(),
            api_key: api_key.map(str::to_owned),
        }
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    pub fn web_origin(&self) -> &str {
        &self.web_origin
    }

    pub fn configured(&self) -> bool {
        self.api_key.is_some()
    }
}

fn normalize_web_origin(origin: String) -> Result<String> {
    let origin = origin.trim().trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&origin).context("MVP_WEB_ORIGIN must be an HTTP origin")?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.path().is_empty() && parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
    {
        bail!("MVP_WEB_ORIGIN must be an exact HTTP origin without a path");
    }
    Ok(origin)
}

#[derive(Clone)]
struct AppState {
    config: Arc<MvpConfig>,
    client: reqwest::Client,
    in_flight: Arc<Semaphore>,
}

pub fn router(config: MvpConfig) -> Result<Router> {
    let allowed_origin = HeaderValue::from_str(config.web_origin())
        .context("MVP_WEB_ORIGIN is not a valid HTTP header value")?;
    let client = reqwest::Client::builder()
        .timeout(PROVIDER_TIMEOUT)
        .build()
        .context("failed to build the MVP provider client")?;
    let state = AppState {
        config: Arc::new(config),
        client,
        in_flight: Arc::new(Semaphore::new(1)),
    };

    let feedback_routes = Router::new()
        .route("/api/mvp/feedback", post(post_feedback))
        .layer(DefaultBodyLimit::max(MULTIPART_BODY_LIMIT))
        // These route layers run before the multipart extractor. In particular,
        // a rejected Origin never causes the upload body to be consumed.
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_origin,
        ))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            enforce_single_flight,
        ));

    let cors = CorsLayer::new()
        .allow_origin(allowed_origin)
        .allow_methods([Method::GET, Method::POST])
        .allow_headers([header::CONTENT_TYPE])
        .expose_headers([header::HeaderName::from_static("x-request-id")]);

    Ok(Router::new()
        .route("/health", get(health))
        .merge(feedback_routes)
        .layer(cors)
        .layer(middleware::from_fn(log_http))
        .with_state(state))
}

async fn log_http(mut request: Request<Body>, next: Next) -> Response {
    let request_id = format!("mvp-{}", NEXT_REQUEST_ID.fetch_add(1, Ordering::Relaxed));
    let method = request.method().clone();
    let uri = request.uri().to_string();
    let origin = request
        .headers()
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    let content_length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned);
    request
        .extensions_mut()
        .insert(RequestId(request_id.clone()));

    let started = Instant::now();
    info!(
        target: "practice_mvp",
        request_id = %request_id,
        method = %method,
        uri = %uri,
        origin = ?origin,
        content_length = ?content_length,
        "HTTP request started"
    );

    let mut response = next.run(request).await;
    let status = response.status();
    let elapsed_ms = started.elapsed().as_millis().min(u64::MAX as u128) as u64;
    response.headers_mut().insert(
        header::HeaderName::from_static("x-request-id"),
        HeaderValue::from_str(&request_id).expect("generated request IDs are valid headers"),
    );

    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        warn!(
            target: "practice_mvp",
            request_id = %request_id,
            method = %method,
            uri = %uri,
            status = %status,
            elapsed_ms,
            "HTTP request completed with error"
        );
    } else {
        info!(
            target: "practice_mvp",
            request_id = %request_id,
            method = %method,
            uri = %uri,
            status = %status,
            elapsed_ms,
            "HTTP request completed"
        );
    }
    response
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    pub category: FindingCategory,
    pub observation: String,
    pub quote: Option<String>,
    pub suggestion: Option<String>,
    pub start_seconds: Option<f64>,
    pub end_seconds: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingCategory {
    Language,
    Delivery,
    Intelligibility,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Feedback {
    pub summary: String,
    pub transcript: String,
    pub strengths: Vec<Finding>,
    pub improvements: Vec<Finding>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssessmentResponse {
    pub feedback: Feedback,
    pub model: String,
    pub elapsed_ms: u64,
    pub usage: Usage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub thought_tokens: Option<u64>,
    pub estimated_cost_usd: Option<f64>,
}

#[derive(Debug)]
pub enum FeedbackValidationError {
    MissingSummary,
    FieldTooLong(&'static str),
    TooManyFindings(&'static str),
    TooManyLimitations,
    EmptyObservation,
}

impl Feedback {
    pub fn validate_and_sanitize(
        &mut self,
        duration: Duration,
    ) -> Result<(), FeedbackValidationError> {
        self.summary = self.summary.trim().to_owned();
        if self.summary.is_empty() {
            return Err(FeedbackValidationError::MissingSummary);
        }
        if self.summary.chars().count() > 600 {
            return Err(FeedbackValidationError::FieldTooLong("summary"));
        }
        if self.transcript.chars().count() > 12_000 {
            return Err(FeedbackValidationError::FieldTooLong("transcript"));
        }
        if self.strengths.len() > 2 {
            return Err(FeedbackValidationError::TooManyFindings("strengths"));
        }
        if self.improvements.len() > 2 {
            return Err(FeedbackValidationError::TooManyFindings("improvements"));
        }
        if self.limitations.len() > 3 {
            return Err(FeedbackValidationError::TooManyLimitations);
        }
        for limitation in &self.limitations {
            if limitation.chars().count() > 300 {
                return Err(FeedbackValidationError::FieldTooLong("limitation"));
            }
        }

        for finding in self
            .strengths
            .iter_mut()
            .chain(self.improvements.iter_mut())
        {
            sanitize_finding(finding, &self.transcript, duration)?;
        }
        Ok(())
    }
}

fn sanitize_finding(
    finding: &mut Finding,
    transcript: &str,
    duration: Duration,
) -> Result<(), FeedbackValidationError> {
    if finding.observation.trim().is_empty() {
        return Err(FeedbackValidationError::EmptyObservation);
    }
    if finding.observation.chars().count() > 800 {
        return Err(FeedbackValidationError::FieldTooLong("observation"));
    }
    if let Some(suggestion) = &finding.suggestion
        && suggestion.chars().count() > 800
    {
        return Err(FeedbackValidationError::FieldTooLong("suggestion"));
    }
    if let Some(quote) = finding.quote.as_mut() {
        *quote = quote.trim().to_owned();
        if quote.is_empty() || quote.chars().count() > 500 || !transcript.contains(quote.as_str()) {
            finding.quote = None;
            finding.start_seconds = None;
            finding.end_seconds = None;
        }
    }

    let valid_range = match (finding.start_seconds, finding.end_seconds) {
        (Some(start), Some(end)) => {
            start.is_finite()
                && end.is_finite()
                && start >= 0.0
                && end >= 0.0
                && start <= end
                && end <= duration.as_secs_f64()
        }
        _ => false,
    };
    if !valid_range {
        finding.start_seconds = None;
        finding.end_seconds = None;
    }
    Ok(())
}

#[derive(Debug)]
struct Submission {
    audio: Vec<u8>,
    mime_type: String,
    duration_ms: u64,
    task: String,
    image: Option<Vec<u8>>,
    image_mime_type: Option<String>,
}

async fn parse_submission(mut multipart: Multipart) -> Result<Submission, MvpError> {
    let mut audio = None;
    let mut mime_type = None;
    let mut duration_ms = None;
    let mut task = None;
    let mut image = None;
    let mut image_mime_type = None;

    while let Some(field) = multipart.next_field().await.map_err(map_multipart_error)? {
        let name = field.name().ok_or(MvpError::InvalidRequest)?.to_owned();
        let bytes = field.bytes().await.map_err(map_multipart_error)?.to_vec();
        match name.as_str() {
            "audio" => {
                if audio.is_some() {
                    return Err(MvpError::InvalidRequest);
                }
                if bytes.is_empty() {
                    return Err(MvpError::InvalidRequest);
                }
                if bytes.len() > MAX_AUDIO_BYTES {
                    return Err(MvpError::TooLarge);
                }
                audio = Some(bytes);
            }
            "mime_type" => {
                if mime_type.is_some() || bytes.len() > 256 {
                    return Err(MvpError::InvalidRequest);
                }
                mime_type = Some(
                    String::from_utf8(bytes)
                        .map_err(|_| MvpError::InvalidRequest)?
                        .trim()
                        .to_owned(),
                );
            }
            "duration_ms" => {
                if duration_ms.is_some() || bytes.len() > 32 {
                    return Err(MvpError::InvalidRequest);
                }
                let value = String::from_utf8(bytes)
                    .map_err(|_| MvpError::InvalidRequest)?
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| MvpError::InvalidRequest)?;
                if !(MIN_DURATION_MS..=MAX_DURATION_MS).contains(&value) {
                    return Err(MvpError::InvalidRequest);
                }
                duration_ms = Some(value);
            }
            "task" => {
                if task.is_some() {
                    return Err(MvpError::InvalidRequest);
                }
                let value = String::from_utf8(bytes)
                    .map_err(|_| MvpError::InvalidRequest)?
                    .trim()
                    .to_owned();
                if value.is_empty() || value.chars().count() > MAX_TASK_CHARS {
                    return Err(MvpError::InvalidRequest);
                }
                task = Some(value);
            }
            "image" => {
                if image.is_some() || bytes.is_empty() {
                    return Err(MvpError::InvalidRequest);
                }
                if bytes.len() > MAX_IMAGE_BYTES {
                    return Err(MvpError::TooLarge);
                }
                image = Some(bytes);
            }
            "image_mime_type" => {
                if image_mime_type.is_some() || bytes.len() > 128 {
                    return Err(MvpError::InvalidRequest);
                }
                image_mime_type = Some(
                    String::from_utf8(bytes)
                        .map_err(|_| MvpError::InvalidRequest)?
                        .trim()
                        .to_owned(),
                );
            }
            _ => return Err(MvpError::InvalidRequest),
        }
    }

    let audio = audio.ok_or(MvpError::InvalidRequest)?;
    let mime_type = mime_type.ok_or(MvpError::InvalidRequest)?;
    let duration_ms = duration_ms.ok_or(MvpError::InvalidRequest)?;
    let task = task.ok_or(MvpError::InvalidRequest)?;

    gemini::provider_mime_type(&mime_type, &audio).map_err(|_| MvpError::UnsupportedAudioType)?;
    if image.is_some() != image_mime_type.is_some() {
        return Err(MvpError::InvalidRequest);
    }
    if let (Some(image), Some(image_mime_type)) = (&image, &image_mime_type) {
        gemini::provider_image_mime_type(image_mime_type, image)
            .map_err(|_| MvpError::UnsupportedImageType)?;
    }

    Ok(Submission {
        audio,
        mime_type,
        duration_ms,
        task,
        image,
        image_mime_type,
    })
}

fn map_multipart_error(error: axum::extract::multipart::MultipartError) -> MvpError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        MvpError::TooLarge
    } else {
        MvpError::InvalidRequest
    }
}

async fn post_feedback(
    State(state): State<AppState>,
    Extension(request_id): Extension<RequestId>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Json<AssessmentResponse>, MvpError> {
    let multipart = multipart.map_err(|_| MvpError::InvalidRequest)?;
    let submission = parse_submission(multipart).await?;
    let api_key = state
        .config
        .api_key
        .as_deref()
        .ok_or(MvpError::MissingKey)?;
    let provider_mime = gemini::provider_mime_type(&submission.mime_type, &submission.audio)
        .map_err(|_| MvpError::UnsupportedAudioType)?;
    info!(
        target: "practice_mvp",
        request_id = %request_id.0,
        audio_bytes = submission.audio.len(),
        client_mime_type = %submission.mime_type,
        provider_mime_type = %provider_mime,
        duration_ms = submission.duration_ms,
        task_chars = submission.task.chars().count(),
        model = %state.config.model(),
        "feedback request accepted; calling Gemini"
    );
    let started = Instant::now();
    let image = submission
        .image
        .as_deref()
        .zip(submission.image_mime_type.as_deref());
    let result = gemini::request_feedback_with_image(
        &state.client,
        api_key,
        state.config.model(),
        &submission.audio,
        provider_mime,
        &submission.task,
        submission.duration_ms,
        image,
        &request_id.0,
    )
    .await
    .map_err(MvpError::from_gemini)?;

    Ok(Json(AssessmentResponse {
        feedback: result.feedback,
        model: result.model,
        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
        usage: result.usage,
    }))
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    Json(json!({
        "status": "ok",
        "model": state.config.model(),
        "configured": state.config.configured(),
    }))
}

async fn enforce_origin(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    if let Some(origin) = request.headers().get(header::ORIGIN)
        && origin.to_str().ok() != Some(state.config.web_origin())
    {
        return MvpError::OriginNotAllowed.into_response();
    }
    next.run(request).await
}

async fn enforce_single_flight(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let permit = match state.in_flight.clone().try_acquire_owned() {
        Ok(permit) => permit,
        Err(_) => return MvpError::Busy.into_response(),
    };
    let response = next.run(request).await;
    drop(permit);
    response
}

#[derive(Debug)]
enum MvpError {
    InvalidRequest,
    TooLarge,
    UnsupportedAudioType,
    UnsupportedImageType,
    OriginNotAllowed,
    Busy,
    MissingKey,
    ProviderLimited,
    Provider,
    InvalidModelOutput,
    Timeout,
}

impl MvpError {
    fn from_gemini(error: gemini::GeminiError) -> Self {
        match error {
            gemini::GeminiError::Timeout => Self::Timeout,
            gemini::GeminiError::ProviderLimited => Self::ProviderLimited,
            gemini::GeminiError::Provider => Self::Provider,
            gemini::GeminiError::InvalidModelOutput => Self::InvalidModelOutput,
        }
    }
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: &'static str,
}

impl IntoResponse for MvpError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            Self::InvalidRequest => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "The feedback request is invalid.",
            ),
            Self::TooLarge => (
                StatusCode::PAYLOAD_TOO_LARGE,
                "too_large",
                "The upload is larger than this prototype allows.",
            ),
            Self::UnsupportedAudioType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_audio_type",
                "Use a WebM/Opus or MP4/M4A recording from the supported capture path.",
            ),
            Self::UnsupportedImageType => (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_image_type",
                "Use a JPEG, PNG, or WebP image within the 5 MiB limit.",
            ),
            Self::OriginNotAllowed => (
                StatusCode::FORBIDDEN,
                "origin_not_allowed",
                "This local endpoint only accepts the configured web origin.",
            ),
            Self::Busy => (
                StatusCode::TOO_MANY_REQUESTS,
                "busy",
                "Another feedback request is already in progress.",
            ),
            Self::MissingKey => (
                StatusCode::SERVICE_UNAVAILABLE,
                "missing_key",
                "GEMINI_API_KEY is not configured for the MVP server.",
            ),
            Self::ProviderLimited => (
                StatusCode::TOO_MANY_REQUESTS,
                "provider_limited",
                "Gemini returned HTTP 429 (rate limit or quota). Check project usage and retry later.",
            ),
            Self::Provider => (
                StatusCode::BAD_GATEWAY,
                "provider_error",
                "Gemini rejected or could not complete the configured request.",
            ),
            Self::InvalidModelOutput => (
                StatusCode::BAD_GATEWAY,
                "invalid_model_output",
                "Gemini returned feedback that did not match the MVP contract.",
            ),
            Self::Timeout => (
                StatusCode::GATEWAY_TIMEOUT,
                "timeout",
                "Gemini took too long. Retry manually; another attempt may incur a charge.",
            ),
        };
        warn!(
            target: "practice_mvp",
            error_code = code,
            status = %status,
            "practice MVP request rejected"
        );
        (
            status,
            Json(ErrorEnvelope {
                error: ErrorDetail { code, message },
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::{body::to_bytes, http::Request};
    use tower::ServiceExt;

    use super::*;

    const BOUNDARY: &str = "mvp-test-boundary";

    fn multipart(audio: &[u8]) -> (String, Vec<u8>) {
        let mut body = Vec::new();
        append_field(&mut body, "audio", audio, Some("audio/webm"));
        append_field(&mut body, "mime_type", b"audio/webm", None);
        append_field(&mut body, "duration_ms", b"20000", None);
        append_field(&mut body, "task", "Cuenta algo real.".as_bytes(), None);
        body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={BOUNDARY}"), body)
    }

    fn append_field(body: &mut Vec<u8>, name: &str, value: &[u8], content_type: Option<&str>) {
        body.extend_from_slice(format!("--{BOUNDARY}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"{name}\"\r\n").as_bytes(),
        );
        if let Some(content_type) = content_type {
            body.extend_from_slice(format!("Content-Type: {content_type}\r\n").as_bytes());
        }
        body.extend_from_slice(b"\r\n");
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }

    async fn response_status(config: MvpConfig, request: Request<Body>) -> StatusCode {
        let response = router(config).unwrap().oneshot(request).await.unwrap();
        response.status()
    }

    #[tokio::test]
    async fn health_does_not_require_a_key() {
        let response = router(MvpConfig::for_tests(None))
            .unwrap()
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().get("x-request-id").is_some());
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["status"], "ok");
        assert_eq!(body["model"], "gemini-3.8-flash");
        assert_eq!(body["configured"], false);
    }

    #[tokio::test]
    async fn wrong_origin_is_rejected_before_upload_processing() {
        let (content_type, body) = multipart(b"not-a-real-recording");
        let request = Request::post("/api/mvp/feedback")
            .header(header::ORIGIN, "http://evil.example")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            response_status(MvpConfig::for_tests(None), request).await,
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn oversized_audio_is_rejected_with_sanitized_error() {
        let audio = vec![0_u8; MAX_AUDIO_BYTES + 1];
        let (content_type, body) = multipart(&audio);
        let request = Request::post("/api/mvp/feedback")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        let response = router(MvpConfig::for_tests(None))
            .unwrap()
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("too_large"));
    }

    #[tokio::test]
    async fn missing_key_is_clear_after_valid_request_parsing() {
        let (content_type, body) = multipart(b"webm bytes");
        let request = Request::post("/api/mvp/feedback")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        let response = router(MvpConfig::for_tests(None))
            .unwrap()
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(String::from_utf8_lossy(&body).contains("missing_key"));
    }

    #[test]
    fn validation_caps_arrays_and_drops_bad_timestamps_or_quotes() {
        let finding = || Finding {
            category: FindingCategory::Language,
            observation: "Observation".to_owned(),
            quote: Some("not in transcript".to_owned()),
            suggestion: None,
            start_seconds: Some(20.0),
            end_seconds: Some(30.0),
        };
        let mut feedback = Feedback {
            summary: "Summary".to_owned(),
            transcript: "sí, esto sí".to_owned(),
            strengths: vec![finding(), finding(), finding()],
            improvements: vec![],
            limitations: vec![],
        };
        assert!(matches!(
            feedback.validate_and_sanitize(Duration::from_secs(60)),
            Err(FeedbackValidationError::TooManyFindings("strengths"))
        ));

        feedback.strengths.truncate(1);
        feedback.strengths[0].quote = Some("sí".to_owned());
        feedback.strengths[0].start_seconds = Some(61.0);
        feedback.strengths[0].end_seconds = Some(62.0);
        feedback
            .validate_and_sanitize(Duration::from_secs(60))
            .unwrap();
        assert_eq!(feedback.strengths[0].quote.as_deref(), Some("sí"));
        assert_eq!(feedback.strengths[0].start_seconds, None);
        assert_eq!(feedback.strengths[0].end_seconds, None);
    }

    #[test]
    fn accepts_bare_http_origins_and_normalizes_the_root_slash() {
        assert_eq!(
            normalize_web_origin("http://localhost:8081/".to_owned()).unwrap(),
            "http://localhost:8081"
        );
        assert!(normalize_web_origin("http://localhost:8081/practice".to_owned()).is_err());
    }
}
