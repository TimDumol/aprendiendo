//! Small, allowlisted telemetry helpers for MCP tool dispatch and recording.
//!
//! Normal action events contain counts, classifications and timings only.
//! Learner-facing text, when present, is emitted as a separate allowlisted
//! event so analytics can exclude it without losing an investigation trail.

use anyhow::{Context, Result, bail};
use std::{env, future::Future, sync::Arc, time::Instant};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

pub const EVENT_VERSION: u8 = 1;

#[derive(Clone, Debug)]
pub struct ServerTelemetry {
    pub process_instance_id: Arc<str>,
}

impl ServerTelemetry {
    pub fn new() -> Self {
        Self {
            process_instance_id: Arc::from(uuid::Uuid::new_v4().to_string()),
        }
    }
}

impl Default for ServerTelemetry {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug)]
pub struct CallContext {
    pub process_instance_id: Arc<str>,
    pub call_id: Arc<str>,
    pub tool: Arc<str>,
}

tokio::task_local! {
    static CALL_CONTEXT: CallContext;
}

pub async fn with_call_context<F, T>(context: CallContext, future: F) -> T
where
    F: Future<Output = T>,
{
    CALL_CONTEXT.scope(context, future).await
}

pub fn current_call_context() -> Option<CallContext> {
    CALL_CONTEXT.try_with(Clone::clone).ok()
}

pub fn emit_started(
    telemetry: &ServerTelemetry,
    call_id: &str,
    tool: &str,
    arguments_json_bytes: Option<usize>,
) {
    match arguments_json_bytes {
        Some(arguments_json_bytes) => tracing::info!(
            target: "aprendiendo_mcp::telemetry",
            event = "mcp_tool_started",
            event_version = EVENT_VERSION,
            process_instance_id = %telemetry.process_instance_id,
            call_id = %call_id,
            tool = %tool,
            arguments_json_bytes,
        ),
        None => tracing::info!(
            target: "aprendiendo_mcp::telemetry",
            event = "mcp_tool_started",
            event_version = EVENT_VERSION,
            process_instance_id = %telemetry.process_instance_id,
            call_id = %call_id,
            tool = %tool,
        ),
    }
}

/// Emit allowlisted learner-facing recording text separately from action
/// metrics. `text_fields_json` is intentionally one field so the default log
/// analyzer can ignore this event class without parsing a potentially large
/// payload. Callers must provide only known recording fields; credentials,
/// headers, identifiers and arbitrary client metadata do not belong here.
pub fn emit_text(telemetry: &ServerTelemetry, call_id: &str, tool: &str, text_fields_json: &str) {
    tracing::info!(
        target: "aprendiendo_mcp::telemetry",
        event = "mcp_tool_text",
        event_version = EVENT_VERSION,
        process_instance_id = %telemetry.process_instance_id,
        call_id = %call_id,
        tool = %tool,
        text_fields_json = %text_fields_json,
    );
}

/// Emit a bounded, structured validation diagnostic without copying the
/// caller's submitted text into metrics. The client-facing error remains
/// precise; this event is for aggregate retry analysis.
pub fn emit_validation_failed(
    telemetry: &ServerTelemetry,
    call_id: &str,
    tool: &str,
    outcome: &str,
    message: &str,
) {
    let diagnostic = validation_diagnostic(outcome, message);
    tracing::warn!(
        target: "aprendiendo_mcp::telemetry",
        event = "mcp_validation_failed",
        event_version = EVENT_VERSION,
        process_instance_id = %telemetry.process_instance_id,
        call_id = %call_id,
        tool = %tool,
        code = diagnostic.code,
        path = (!diagnostic.path.is_empty()).then_some(diagnostic.path),
        expected = diagnostic.expected,
        correction = diagnostic.correction,
    );
}

/// The structured diagnostic shared by client-facing recording errors and
/// telemetry. It deliberately contains only schema facts; validation messages
/// are never copied verbatim because a future validator may include submitted
/// learner text in its prose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationDiagnostic {
    pub code: &'static str,
    pub path: String,
    pub expected: String,
    pub correction: String,
}

fn detail_without_prefix(message: &str) -> &str {
    let trimmed = message.trim();
    for prefix in [
        "invalid_argument:",
        "unknown_reference:",
        "failed to deserialize parameters:",
        "Failed to parse parameters:",
    ] {
        if let Some(detail) = trimmed.strip_prefix(prefix) {
            return detail.trim();
        }
    }
    trimmed
}

fn quoted_field(detail: &str, marker: &str) -> Option<String> {
    let start = detail.find(marker)? + marker.len();
    let rest = &detail[start..];
    let quote = rest.chars().next()?;
    if !matches!(quote, '\'' | '`' | '"') {
        return None;
    }
    let end = rest[quote.len_utf8()..].find(quote)? + quote.len_utf8();
    Some(rest[quote.len_utf8()..end].to_owned())
}

fn diagnostic_path(detail: &str) -> String {
    if detail.starts_with("reviews weakness_key") {
        return "reviews[].weakness_key".into();
    }
    if detail.starts_with("reviews target") {
        return "reviews[].target".into();
    }
    if detail.starts_with("a session") {
        return "session".into();
    }
    if detail.starts_with("serialized practice session") {
        return "request".into();
    }
    if detail.starts_with("only one FSRS review") {
        return "reviews".into();
    }
    if let Some(field) =
        quoted_field(detail, "unknown field ").or_else(|| quoted_field(detail, "missing field "))
    {
        return field;
    }
    let first = detail.split_whitespace().next().unwrap_or_default();
    const ROOT_FIELDS: &[&str] = &[
        "idempotency_key",
        "session_date",
        "reviewed_at",
        "task_ref",
        "exercise_type_key",
        "topic",
        "notes",
        "policy_version",
        "new_weaknesses",
        "items",
        "attempts",
        "observations",
        "findings",
        "reviews",
        "activity_runs",
        "session_id",
        "limit",
        "skill",
        "from_date",
        "to_date",
        "evidence",
        "timing_source",
    ];
    if first.contains('.') || first.contains('[') || ROOT_FIELDS.contains(&first) {
        return first
            .trim_matches(|c: char| c == '`' || c == '\'' || c == '"' || c == ':')
            .to_owned();
    }
    String::new()
}

fn expected_description(detail: &str) -> String {
    if detail.contains("must not be blank") || detail.contains("must not be empty") {
        "nonblank text".into()
    } else if detail.contains("must contain 1") {
        "nonblank text within the stated limit".into()
    } else if detail.contains("may contain at most") {
        "a collection no larger than the stated limit".into()
    } else if detail.contains("must be at most") {
        "a value within the stated size limit".into()
    } else if detail.contains("must be positive") || detail.contains("must be >= 1") {
        "a positive number".into()
    } else if detail.contains("must be unique") {
        "a positive value unique within this request".into()
    } else if detail.contains("references") {
        "an existing reference in this request".into()
    } else if detail.contains("required") {
        "the required companion field".into()
    } else if detail.contains("unknown field") {
        "a field declared by the tool schema".into()
    } else if let Some(expected) = detail.split("expected ").nth(1) {
        expected
            .split([',', ';'])
            .next()
            .unwrap_or(expected)
            .trim()
            .to_owned()
    } else {
        "a value satisfying the field constraint".into()
    }
}

/// Convert a bounded validator outcome and message into an actionable,
/// machine-filterable diagnostic. The resulting text is safe to return to the
/// caller and safe to include in aggregate troubleshooting output.
pub fn validation_diagnostic(outcome: &str, message: &str) -> ValidationDiagnostic {
    let code = if outcome == "unknown_reference" {
        "unknown_reference"
    } else {
        "invalid_argument"
    };
    let detail = detail_without_prefix(message);
    let path = diagnostic_path(detail);
    let expected = expected_description(detail);
    let correction = if code == "unknown_reference" {
        if path.is_empty() {
            "use an existing reference from this request or remove the field".into()
        } else {
            format!("set {path} to an existing reference from this request or remove the field")
        }
    } else if path.is_empty() {
        "correct the request according to the tool schema and retry".into()
    } else {
        format!(
            "correct {path} and retry; keep the same idempotency key only for an identical request"
        )
    };
    ValidationDiagnostic {
        code,
        path,
        expected,
        correction,
    }
}

pub fn format_validation_error(outcome: &str, message: &str) -> String {
    let diagnostic = validation_diagnostic(outcome, message);
    let path = if diagnostic.path.is_empty() {
        "request"
    } else {
        diagnostic.path.as_str()
    };
    format!(
        "{}: field={path}; expected={}; correction={}",
        diagnostic.code, diagnostic.expected, diagnostic.correction
    )
}

#[derive(Clone, Debug, Default)]
pub struct CompletionMetrics {
    pub outcome: &'static str,
    pub duration_ms: u64,
    pub arguments_json_bytes: Option<usize>,
    pub response_json_bytes: Option<usize>,
    pub record_status: Option<&'static str>,
    pub session_id: Option<i64>,
    pub item_count: Option<usize>,
    pub attempt_count: Option<usize>,
    pub observation_count: Option<usize>,
    pub new_weakness_count: Option<usize>,
    pub review_applied_count: Option<usize>,
    pub review_skipped_count: Option<usize>,
}

pub fn emit_completed(
    telemetry: &ServerTelemetry,
    call_id: &str,
    tool: &str,
    metrics: &CompletionMetrics,
) {
    tracing::info!(
        target: "aprendiendo_mcp::telemetry",
        event = "mcp_tool_completed",
        event_version = EVENT_VERSION,
        process_instance_id = %telemetry.process_instance_id,
        call_id = %call_id,
        tool = %tool,
        arguments_json_bytes = metrics.arguments_json_bytes,
        response_json_bytes = metrics.response_json_bytes,
        duration_ms = metrics.duration_ms,
        outcome = metrics.outcome,
        record_status = metrics.record_status,
        session_id = metrics.session_id,
        item_count = metrics.item_count,
        attempt_count = metrics.attempt_count,
        observation_count = metrics.observation_count,
        new_weakness_count = metrics.new_weakness_count,
        review_applied_count = metrics.review_applied_count,
        review_skipped_count = metrics.review_skipped_count,
    );
}

pub fn emit_phase(phase: &'static str, duration_ms: u64) {
    let Some(context) = current_call_context() else {
        return;
    };
    emit_phase_for(&context, phase, duration_ms);
}

fn emit_phase_for(context: &CallContext, phase: &'static str, duration_ms: u64) {
    tracing::info!(
        target: "aprendiendo_mcp::telemetry",
        event = "mcp_recording_phase",
        event_version = EVENT_VERSION,
        process_instance_id = %context.process_instance_id,
        call_id = %context.call_id,
        tool = %context.tool,
        phase,
        duration_ms,
    );
}

pub struct PhaseTimer {
    phase: &'static str,
    started: Instant,
}

impl PhaseTimer {
    pub fn new(phase: &'static str) -> Self {
        Self {
            phase,
            started: Instant::now(),
        }
    }
}

impl Drop for PhaseTimer {
    fn drop(&mut self) {
        emit_phase(self.phase, self.started.elapsed().as_millis() as u64);
    }
}

/// Initialize the application subscriber. JSON is the documented production
/// format; text remains useful for local development. Validation happens at
/// startup so a typo cannot silently disable the analytics contract.
pub fn init_tracing() -> Result<()> {
    let format = env::var("LOG_FORMAT").unwrap_or_else(|_| "json".to_owned());
    if !matches!(format.as_str(), "json" | "text") {
        bail!("unsupported LOG_FORMAT {format:?}; expected json or text");
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "aprendiendo_mcp=info,tower_http=info".into());
    if format == "json" {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .json()
                    .flatten_event(true)
                    .with_ansi(false)
                    .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339()),
            )
            .try_init()
            .context("initialize JSON tracing subscriber")?;
    } else {
        tracing_subscriber::registry()
            .with(filter)
            .with(
                tracing_subscriber::fmt::layer()
                    .with_ansi(false)
                    .with_timer(tracing_subscriber::fmt::time::UtcTime::rfc_3339()),
            )
            .try_init()
            .context("initialize text tracing subscriber")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};
    use tracing_subscriber::{fmt::MakeWriter, layer::SubscriberExt};

    #[derive(Clone)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    struct SharedGuard(Arc<Mutex<Vec<u8>>>);

    impl io::Write for SharedGuard {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("telemetry buffer should not be poisoned")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl<'a> MakeWriter<'a> for SharedWriter {
        type Writer = SharedGuard;

        fn make_writer(&'a self) -> Self::Writer {
            SharedGuard(self.0.clone())
        }
    }

    #[tokio::test]
    async fn events_are_versioned_and_allowlisted() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_ansi(false)
                .with_writer(SharedWriter(buffer.clone())),
        );
        let telemetry = ServerTelemetry::new();
        let context = CallContext {
            process_instance_id: telemetry.process_instance_id.clone(),
            call_id: Arc::from("call-test"),
            tool: Arc::from("record_tutoring_session"),
        };
        tracing::subscriber::with_default(subscriber, || {
            emit_started(&telemetry, "call-test", "record_tutoring_session", Some(42));
            emit_phase_for(&context, "compact_expansion", 3);
            emit_completed(
                &telemetry,
                "call-test",
                "record_tutoring_session",
                &CompletionMetrics {
                    outcome: "success",
                    duration_ms: 7,
                    arguments_json_bytes: Some(42),
                    response_json_bytes: Some(128),
                    record_status: Some("created"),
                    session_id: Some(8),
                    item_count: Some(2),
                    attempt_count: Some(3),
                    observation_count: Some(4),
                    new_weakness_count: Some(1),
                    review_applied_count: Some(0),
                    review_skipped_count: Some(0),
                },
            );
        });

        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        let records = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records.len(), 3);
        assert!(
            records.iter().all(|record| {
                record["event_version"] == EVENT_VERSION
                    && record["process_instance_id"] == telemetry.process_instance_id.as_ref()
                    && record["call_id"] == "call-test"
                    && record["tool"] == "record_tutoring_session"
            }),
            "{output}"
        );
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "mcp_tool_started")
        );
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "mcp_recording_phase")
        );
        assert!(
            records
                .iter()
                .any(|record| record["event"] == "mcp_tool_completed")
        );
        let completed = records
            .iter()
            .find(|record| record["event"] == "mcp_tool_completed")
            .unwrap();
        assert_eq!(completed["response_json_bytes"], 128);
        assert!(!output.contains("transcript"));
        assert!(!output.contains("idempotency_key"));
    }

    #[test]
    fn text_and_validation_events_are_separate_and_structured() {
        let buffer = Arc::new(Mutex::new(Vec::new()));
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .json()
                .flatten_event(true)
                .with_ansi(false)
                .with_writer(SharedWriter(buffer.clone())),
        );
        let telemetry = ServerTelemetry::new();
        tracing::subscriber::with_default(subscriber, || {
            emit_text(
                &telemetry,
                "call-text",
                "record_tutoring_session",
                r#"[{"path":"turns[0].attempts[0].transcript","text":"learner sentinel"}]"#,
            );
            emit_validation_failed(
                &telemetry,
                "call-invalid",
                "record_tutoring_session",
                "invalid_argument",
                "turns[0].attempts[0].transcript must not be blank",
            );
        });
        let output = String::from_utf8(buffer.lock().unwrap().clone()).unwrap();
        let records = output
            .lines()
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        assert_eq!(records[0]["event"], "mcp_tool_text");
        assert!(
            records[0]["text_fields_json"]
                .as_str()
                .unwrap()
                .contains("learner sentinel")
        );
        assert_eq!(records[1]["event"], "mcp_validation_failed");
        assert_eq!(records[1]["code"], "invalid_argument");
        assert_eq!(records[1]["path"], "turns[0].attempts[0].transcript");
        assert_eq!(records[1]["expected"], "nonblank text");
        assert!(records[1]["correction"].is_string());
    }

    #[test]
    fn diagnostics_extract_schema_fields_without_echoing_submitted_values() {
        let unknown = validation_diagnostic(
            "invalid_argument",
            "failed to deserialize parameters: unknown field `unexpected_field`, expected `idempotency_key`",
        );
        assert_eq!(unknown.code, "invalid_argument");
        assert_eq!(unknown.path, "unexpected_field");
        assert_eq!(unknown.expected, "a field declared by the tool schema");
        assert!(!unknown.correction.contains("learner-secret"));

        let reference = validation_diagnostic(
            "unknown_reference",
            "unknown_reference: findings[0].attempt_no references an attempt not in this request",
        );
        assert_eq!(reference.path, "findings[0].attempt_no");
        assert_eq!(reference.expected, "an existing reference in this request");
        assert!(
            format_validation_error(
                "unknown_reference",
                "findings[0].attempt_no references an attempt not in this request"
            )
            .contains("field=findings[0].attempt_no")
        );
    }
}
