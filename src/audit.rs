//! Offline, source-backed historical data audit and amendment support.
//!
//! The MCP recording path remains deterministic.  This module is deliberately
//! an operator-facing boundary: it exports a bounded SQLite snapshot, validates
//! typed proposals, and applies them atomically with optimistic concurrency.

use anyhow::{Context, Result, anyhow, bail};
use chrono::{NaiveDate, SecondsFormat, Utc};
use chrono_tz::Europe::Madrid;
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::Path;

pub const AUDIT_SCHEMA_VERSION: u16 = 1;
pub const TABLES_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS audit_runs (
  audit_run_id TEXT PRIMARY KEY,
  snapshot_boundary TEXT NOT NULL,
  snapshot_hash TEXT NOT NULL,
  model_id TEXT,
  prompt_version TEXT,
  scope_json TEXT NOT NULL DEFAULT '{}',
  started_at TEXT NOT NULL,
  ended_at TEXT,
  status TEXT NOT NULL CHECK(status IN ('planned','running','completed','failed','partial')),
  records_examined INTEGER NOT NULL DEFAULT 0 CHECK(records_examined >= 0),
  proposed_count INTEGER NOT NULL DEFAULT 0 CHECK(proposed_count >= 0),
  applied_count INTEGER NOT NULL DEFAULT 0 CHECK(applied_count >= 0),
  rejected_count INTEGER NOT NULL DEFAULT 0 CHECK(rejected_count >= 0),
  disputed_count INTEGER NOT NULL DEFAULT 0 CHECK(disputed_count >= 0),
  failure_count INTEGER NOT NULL DEFAULT 0 CHECK(failure_count >= 0),
  latest_successful_boundary TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE TABLE IF NOT EXISTS data_amendments (
  audit_run_id TEXT NOT NULL REFERENCES audit_runs(audit_run_id),
  proposal_id TEXT NOT NULL,
  entity_type TEXT NOT NULL,
  entity_id TEXT NOT NULL,
  expected_revision INTEGER NOT NULL CHECK(expected_revision >= 1),
  before_hash TEXT NOT NULL,
  operation TEXT NOT NULL,
  before_json TEXT NOT NULL CHECK(json_valid(before_json)),
  after_json TEXT NOT NULL CHECK(json_valid(after_json)),
  source_refs_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(source_refs_json)),
  reason TEXT NOT NULL,
  issue_class TEXT NOT NULL,
  certainty TEXT NOT NULL CHECK(certainty IN ('supported','disputed','insufficient_evidence')),
  reviewer TEXT NOT NULL,
  prompt_version TEXT NOT NULL,
  reviewed_at TEXT NOT NULL,
  dependencies_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(dependencies_json)),
  affected_consumers_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(affected_consumers_json)),
  predecessor_revision INTEGER NOT NULL CHECK(predecessor_revision >= 1),
  status TEXT NOT NULL CHECK(status IN ('proposed','applied','rejected','disputed','reverted')),
  applied_at TEXT,
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  error TEXT,
  PRIMARY KEY(audit_run_id, proposal_id)
);
CREATE INDEX IF NOT EXISTS data_amendments_entity_idx
  ON data_amendments(entity_type, entity_id, created_at);
CREATE INDEX IF NOT EXISTS data_amendments_run_status_idx
  ON data_amendments(audit_run_id, status);
CREATE TABLE IF NOT EXISTS effective_summaries (
  session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  summary_key TEXT NOT NULL,
  summary_json TEXT NOT NULL CHECK(json_valid(summary_json)),
  revision INTEGER NOT NULL CHECK(revision >= 1),
  updated_at TEXT NOT NULL,
  PRIMARY KEY(session_id, summary_key)
);
CREATE TABLE IF NOT EXISTS learner_goals (
  id INTEGER PRIMARY KEY CHECK(id=1),
  exam_key TEXT NOT NULL,
  target_date TEXT,
  oral_date TEXT,
  timezone TEXT NOT NULL,
  source TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS exam_specifications (
  exam_key TEXT NOT NULL,
  specification_version TEXT NOT NULL,
  source_url TEXT NOT NULL,
  retrieved_at TEXT NOT NULL,
  config_json TEXT NOT NULL CHECK(json_valid(config_json) AND json_type(config_json)='object'),
  PRIMARY KEY(exam_key, specification_version)
);
CREATE TABLE IF NOT EXISTS task_requirements (
  id INTEGER PRIMARY KEY,
  session_id INTEGER REFERENCES sessions(id) ON DELETE CASCADE,
  practice_item_id INTEGER,
  exam_key TEXT,
  specification_version TEXT,
  skill TEXT NOT NULL,
  task_key TEXT NOT NULL,
  prompt_hash TEXT,
  requested_points_json TEXT NOT NULL DEFAULT '[]' CHECK(json_valid(requested_points_json)),
  word_min INTEGER CHECK(word_min IS NULL OR word_min >= 0),
  word_max INTEGER CHECK(word_max IS NULL OR word_max >= word_min),
  time_budget_seconds INTEGER CHECK(time_budget_seconds IS NULL OR time_budget_seconds > 0),
  preparation_budget_seconds INTEGER CHECK(preparation_budget_seconds IS NULL OR preparation_budget_seconds > 0),
  source TEXT NOT NULL CHECK(source IN ('official','teacher_created','micropractice')),
  created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
  FOREIGN KEY (practice_item_id, session_id) REFERENCES practice_items(id, session_id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS task_requirements_item_idx
  ON task_requirements(practice_item_id, session_id);
CREATE TABLE IF NOT EXISTS requested_point_assessments (
  id INTEGER PRIMARY KEY,
  attempt_id INTEGER NOT NULL REFERENCES attempts(id) ON DELETE CASCADE,
  point_key TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('met','partial','missing','unclear')),
  supporting_excerpt TEXT,
  reason TEXT,
  evaluator TEXT NOT NULL,
  rubric_version TEXT,
  assessed_at TEXT NOT NULL,
  retrospective INTEGER NOT NULL DEFAULT 0 CHECK(retrospective IN (0,1)),
  UNIQUE(attempt_id, point_key)
);
CREATE TABLE IF NOT EXISTS audit_checkpoints (
  audit_run_id TEXT NOT NULL REFERENCES audit_runs(audit_run_id) ON DELETE CASCADE,
  unit_key TEXT NOT NULL,
  source_hash TEXT NOT NULL,
  status TEXT NOT NULL CHECK(status IN ('pending','completed','failed')),
  error TEXT,
  updated_at TEXT NOT NULL,
  PRIMARY KEY(audit_run_id, unit_key)
);
"#;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AuditOperation {
    AmendAssessment,
    RetractObservation,
    ReclassifyFinding,
    LinkContinuation,
    MarkDuplicate,
    ReassignTarget,
    AddRecoveredFinding,
    InvalidateReview,
    AmendSummary,
}

impl AuditOperation {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AmendAssessment => "amend_assessment",
            Self::RetractObservation => "retract_observation",
            Self::ReclassifyFinding => "reclassify_finding",
            Self::LinkContinuation => "link_continuation",
            Self::MarkDuplicate => "mark_duplicate",
            Self::ReassignTarget => "reassign_target",
            Self::AddRecoveredFinding => "add_recovered_finding",
            Self::InvalidateReview => "invalidate_review",
            Self::AmendSummary => "amend_summary",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Certainty {
    Supported,
    Disputed,
    InsufficientEvidence,
}

impl Certainty {
    fn as_str(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Disputed => "disputed",
            Self::InsufficientEvidence => "insufficient_evidence",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SourceRef {
    pub source_id: String,
    pub excerpt: Option<String>,
    pub span: Option<String>,
    pub source_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuditProposal {
    pub audit_run_id: String,
    pub proposal_id: String,
    pub schema_version: u16,
    pub entity_type: String,
    pub entity_id: String,
    pub expected_revision: i64,
    pub before_hash: String,
    pub operation: AuditOperation,
    /// The operator-facing operation is typed and the affected JSON fields are
    /// checked against an allowlist below.  Arbitrary SQL or JSON paths are
    /// never accepted.
    pub before: Value,
    pub after: Value,
    pub source_refs: Vec<SourceRef>,
    pub reason: String,
    pub issue_class: String,
    pub certainty: Certainty,
    pub reviewer: String,
    pub prompt_version: String,
    pub reviewed_at: String,
    #[serde(default)]
    pub dependencies: Vec<String>,
    #[serde(default)]
    pub affected_consumers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AuditBatch {
    pub audit_run_id: String,
    pub snapshot_boundary: String,
    pub snapshot_hash: String,
    pub model_id: Option<String>,
    pub prompt_version: String,
    pub scope: Value,
    pub proposals: Vec<AuditProposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportManifest {
    pub snapshot_path: String,
    pub snapshot_hash: String,
    pub snapshot_boundary: String,
    pub schema_version: String,
    pub generated_at: String,
    pub counts: BTreeMap<String, i64>,
    pub available_sources: Vec<String>,
    pub missing_sources: Vec<String>,
}

pub fn install(tx: &Transaction<'_>) -> Result<()> {
    for (table, column, sql) in [
        (
            "sessions",
            "exercise_instance_id",
            "ALTER TABLE sessions ADD COLUMN exercise_instance_id TEXT",
        ),
        (
            "sessions",
            "stimulus_identity",
            "ALTER TABLE sessions ADD COLUMN stimulus_identity TEXT",
        ),
        (
            "sessions",
            "source_event_id",
            "ALTER TABLE sessions ADD COLUMN source_event_id TEXT",
        ),
        (
            "sessions",
            "revision",
            "ALTER TABLE sessions ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision >= 1)",
        ),
        (
            "practice_items",
            "effective_status",
            "ALTER TABLE practice_items ADD COLUMN effective_status TEXT NOT NULL DEFAULT 'active' CHECK(effective_status IN ('active','retracted','disputed'))",
        ),
        (
            "practice_items",
            "revision",
            "ALTER TABLE practice_items ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision >= 1)",
        ),
        (
            "practice_items",
            "computed_word_count",
            "ALTER TABLE practice_items ADD COLUMN computed_word_count INTEGER CHECK(computed_word_count IS NULL OR computed_word_count >= 0)",
        ),
        (
            "practice_items",
            "word_counter_version",
            "ALTER TABLE practice_items ADD COLUMN word_counter_version TEXT",
        ),
        (
            "observations",
            "effective_status",
            "ALTER TABLE observations ADD COLUMN effective_status TEXT NOT NULL DEFAULT 'active' CHECK(effective_status IN ('active','retracted','disputed'))",
        ),
        (
            "observations",
            "revision",
            "ALTER TABLE observations ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision >= 1)",
        ),
        (
            "session_findings",
            "effective_status",
            "ALTER TABLE session_findings ADD COLUMN effective_status TEXT NOT NULL DEFAULT 'active' CHECK(effective_status IN ('active','retracted','disputed'))",
        ),
        (
            "session_findings",
            "revision",
            "ALTER TABLE session_findings ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision >= 1)",
        ),
        (
            "weakness_reviews",
            "effective_status",
            "ALTER TABLE weakness_reviews ADD COLUMN effective_status TEXT NOT NULL DEFAULT 'active' CHECK(effective_status IN ('active','invalidated','disputed'))",
        ),
        (
            "weakness_reviews",
            "revision",
            "ALTER TABLE weakness_reviews ADD COLUMN revision INTEGER NOT NULL DEFAULT 1 CHECK(revision >= 1)",
        ),
    ] {
        if !column_exists(tx, table, column)? {
            tx.execute(sql, [])?;
        }
    }
    tx.execute_batch(TABLES_DDL)?;
    tx.execute_batch("CREATE UNIQUE INDEX IF NOT EXISTS sessions_source_event_idx ON sessions(source_event_id) WHERE source_event_id IS NOT NULL;")?;
    tx.execute_batch("CREATE INDEX IF NOT EXISTS observations_effective_idx ON observations(effective_status, weakness_id, session_id); CREATE INDEX IF NOT EXISTS session_findings_effective_idx ON session_findings(effective_status, session_id, id); CREATE INDEX IF NOT EXISTS weakness_reviews_effective_idx ON weakness_reviews(effective_status, scheduler_item_id, reviewed_at);")?;
    let legacy_items = tx
        .prepare("SELECT id,response FROM practice_items WHERE response IS NOT NULL AND computed_word_count IS NULL")?
        .query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, response) in legacy_items {
        tx.execute(
            "UPDATE practice_items SET computed_word_count=?1,word_counter_version=?2 WHERE id=?3",
            params![
                i64::try_from(word_count(&response))?,
                word_counter_version(),
                id
            ],
        )?;
    }
    tx.execute("INSERT OR IGNORE INTO learner_goals(id,exam_key,target_date,oral_date,timezone,source,updated_at) VALUES(1,'dele_a2','2026-10-17',NULL,'Europe/Madrid','plan:data-quality-and-dele-a2:2026-09-12',strftime('%Y-%m-%dT%H:%M:%fZ','now'))", [])?;
    tx.execute("INSERT OR IGNORE INTO exam_specifications(exam_key,specification_version,source_url,retrieved_at,config_json) VALUES('dele_a2','2022-guide-v2020','https://examenes.cervantes.es/sites/default/files/DELE_A2_v2020_Gu%C3%ADa_de_examen_0.pdf','2026-09-12',?1)", [official_exam_config().to_string()])?;
    Ok(())
}

fn column_exists(c: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = c.prepare(&format!(
        "PRAGMA table_info(\"{}\")",
        table.replace('"', "\"\"")
    ))?;
    Ok(stmt
        .query_map([], |r| r.get::<_, String>(1))?
        .any(|v| v.as_deref() == Ok(column)))
}

pub fn official_exam_config() -> Value {
    json!({
        "exam_key": "dele_a2",
        "specification_version": "2022-guide-v2020",
        "source_url": "https://examenes.cervantes.es/sites/default/files/DELE_A2_v2020_Gu%C3%ADa_de_examen_0.pdf",
        "tests": {
            "reading": {"duration_minutes": 60, "tasks": 4, "items": 25, "scoring_group": "reading_writing"},
            "listening": {"duration_minutes": 40, "tasks": 4, "items": 25, "scoring_group": "listening_speaking"},
            "writing": {"duration_minutes": 45, "tasks": 2, "scoring_group": "reading_writing", "tasks_detail": {
                "writing_task_1": {"word_min": 60, "word_max": 70, "weight_within_test_percent": 50},
                "writing_task_2": {"word_min": 70, "word_max": 80, "weight_within_test_percent": 50}
            }},
            "speaking": {"duration_minutes": 12, "preparation_minutes": 12, "tasks": 3, "scoring_group": "listening_speaking", "tasks_detail": {
                "oral_task_1": {"label": "personal presentation"},
                "oral_task_2": {"label": "picture description"},
                "oral_task_3": {"label": "simulated conversation"}
            }}
        },
        "scoring_groups": {
            "reading_writing": {"required_points": 30, "out_of": 50},
            "listening_speaking": {"required_points": 30, "out_of": 50}
        },
        "constraints_label": "Official DELE constraints; application task/word count conventions remain local metadata."
    })
}

pub fn exam_task_constraints(task: &str) -> Option<Value> {
    let value = match task {
        "reading" => {
            json!({"task_key":"reading","skill":"reading","duration_seconds":3600,"task_count":4,"item_count":25,"official":true})
        }
        "listening" => {
            json!({"task_key":"listening","skill":"listening","duration_seconds":2400,"task_count":4,"item_count":25,"official":true,"modality_required":"audio_source"})
        }
        "full_writing" => {
            json!({"task_key":"full_writing","skill":"writing","duration_seconds":2700,"task_count":2,"word_ranges":[{"task":"writing_task_1","min":60,"max":70},{"task":"writing_task_2","min":70,"max":80}],"official":true})
        }
        "writing_task_1" => {
            json!({"task_key":"writing_task_1","skill":"writing","test_duration_seconds":2700,"duration_seconds":null,"word_min":60,"word_max":70,"official":true})
        }
        "writing_task_2" => {
            json!({"task_key":"writing_task_2","skill":"writing","test_duration_seconds":2700,"duration_seconds":null,"word_min":70,"word_max":80,"official":true})
        }
        "full_oral" => {
            json!({"task_key":"full_oral","skill":"speaking","duration_seconds":720,"preparation_seconds":720,"task_count":3,"official":true})
        }
        "oral_task_1" => {
            json!({"task_key":"oral_task_1","skill":"speaking","duration_min_seconds":120,"duration_max_seconds":180,"test_duration_seconds":720,"shared_preparation_seconds":720,"official":true})
        }
        "oral_task_2" => {
            json!({"task_key":"oral_task_2","skill":"speaking","duration_min_seconds":120,"duration_max_seconds":180,"test_duration_seconds":720,"shared_preparation_seconds":720,"official":true})
        }
        "oral_task_3" => {
            json!({"task_key":"oral_task_3","skill":"speaking","duration_min_seconds":180,"duration_max_seconds":240,"test_duration_seconds":720,"shared_preparation_seconds":720,"official":true})
        }
        _ => return None,
    };
    Some(value)
}

pub fn learner_goal(c: &Connection) -> Result<Value> {
    let row: Option<(String, Option<String>, Option<String>, String, String)> = c
        .query_row(
            "SELECT exam_key,target_date,oral_date,timezone,source FROM learner_goals WHERE id=1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .optional()?;
    let Some((exam_key, target_date, oral_date, timezone, source)) = row else {
        return Ok(json!({"status":"unknown"}));
    };
    let today = Utc::now().with_timezone(&Madrid).date_naive();
    let days_remaining = target_date
        .as_deref()
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        .map(|target| (target - today).num_days());
    Ok(json!({
        "exam_key": exam_key,
        "target_date": target_date,
        "oral_date": oral_date,
        "timezone": timezone,
        "today": today.to_string(),
        "days_remaining": days_remaining,
        "source": source,
        "oral_date_status": if oral_date.is_some() { "supplied" } else { "unknown" }
    }))
}

pub fn exam_progress(c: &Connection) -> Result<Value> {
    let mut skills = serde_json::Map::new();
    for skill in ["reading", "listening", "writing", "speaking"] {
        let task_count: i64 = c.query_row(
            "SELECT count(*) FROM task_requirements WHERE skill=?1 AND source='official'",
            [skill],
            |r| r.get(0),
        )?;
        let assessment_count: i64 = c.query_row(
            "SELECT count(*) FROM requested_point_assessments r JOIN attempts a ON a.id=r.attempt_id WHERE EXISTS(SELECT 1 FROM task_requirements t WHERE t.session_id=a.session_id AND t.skill=?1 AND (t.practice_item_id=a.practice_item_id OR t.practice_item_id IS NULL)) AND (a.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=a.practice_item_id AND p.effective_status='active'))",
            [skill],
            |r| r.get(0),
        )?;
        skills.insert(
            skill.to_owned(),
            json!({
                "status": if assessment_count > 0 { "assessments_recorded" } else if task_count > 0 { "requirements_present_no_assessments" } else { "unknown" },
                "official_task_requirements": task_count,
                "assessed_requested_points": assessment_count,
                "denominator": "Only explicitly task-scoped assessed opportunities are eligible; incidental findings are excluded."
            }),
        );
    }
    Ok(json!({
        "goal": learner_goal(c)?,
        "skills": skills,
        "scoring_groups": official_exam_config()["scoring_groups"].clone(),
        "next_priorities": crate::production::get(c)?["preferences"]["learning_focus"]["skills"],
        "readiness": "unknown_until_compatible_component_evidence_exists"
    }))
}

/// Shared effective-data summary for context, briefs, queues and offline reports.
/// Every count is derived from the current effective projection; raw history is
/// still available through the audit export.
pub fn effective_evidence_summary(c: &Connection) -> Result<Value> {
    let unique_source_events: i64 = c.query_row(
        "SELECT count(DISTINCT COALESCE(source_event_id,'session:'||id)) FROM sessions",
        [],
        |r| r.get(0),
    )?;
    let unique_exercise_instances: i64 = c.query_row(
        "SELECT count(DISTINCT COALESCE(exercise_instance_id,'session:'||id)) FROM sessions",
        [],
        |r| r.get(0),
    )?;
    let unique_materials: i64 = c.query_row(
        "SELECT count(DISTINCT stimulus_identity) FROM sessions WHERE stimulus_identity IS NOT NULL",
        [],
        |r| r.get(0),
    )?;
    let unique_dates: i64 = c.query_row(
        "SELECT count(DISTINCT session_date) FROM sessions",
        [],
        |r| r.get(0),
    )?;
    let active_attempts: i64 = c.query_row(
        "SELECT count(*) FROM attempts a WHERE a.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=a.practice_item_id AND p.effective_status='active')",
        [],
        |r| r.get(0),
    )?;
    let active_observations: i64 = c.query_row(
        "SELECT count(*) FROM observations WHERE effective_status='active'",
        [],
        |r| r.get(0),
    )?;
    let disputed_observations: i64 = c.query_row(
        "SELECT count(*) FROM observations WHERE effective_status='disputed'",
        [],
        |r| r.get(0),
    )?;
    let retracted_observations: i64 = c.query_row(
        "SELECT count(*) FROM observations WHERE effective_status='retracted'",
        [],
        |r| r.get(0),
    )?;
    let active_findings: i64 = c.query_row(
        "SELECT count(*) FROM session_findings WHERE effective_status='active'",
        [],
        |r| r.get(0),
    )?;
    let disputed_findings: i64 = c.query_row(
        "SELECT count(*) FROM session_findings WHERE effective_status='disputed'",
        [],
        |r| r.get(0),
    )?;
    let active_reviews: i64 = c.query_row(
        "SELECT count(*) FROM weakness_reviews WHERE effective_status='active'",
        [],
        |r| r.get(0),
    )?;
    let invalidated_reviews: i64 = c.query_row(
        "SELECT count(*) FROM weakness_reviews WHERE effective_status='invalidated'",
        [],
        |r| r.get(0),
    )?;

    let mut evidence_classes = BTreeMap::<String, i64>::new();
    let mut outcomes = BTreeMap::<String, BTreeMap<String, i64>>::new();
    let mut last_independent_date = None::<String>;
    let mut stmt = c.prepare(
        "SELECT s.session_date,ps.audit_json FROM production_sessions ps JOIN sessions s ON s.id=ps.session_id ORDER BY s.session_date,s.id",
    )?;
    for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (session_date, raw) = row?;
        let audit: Value = serde_json::from_str(&raw)?;
        let Some(turns) = audit.get("turns").and_then(Value::as_array) else {
            continue;
        };
        for turn in turns {
            let base_class = turn
                .get("observed_evidence_class")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let attempt_kind = turn
                .get("attempt_kind")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let class = match (base_class, attempt_kind) {
                ("assisted", _) => "assisted_repair",
                ("independent", "fresh_transfer") => "supported_delayed_independent_transfer",
                ("independent", "initial") => {
                    last_independent_date = Some(session_date.clone());
                    "independent_first_attempt"
                }
                ("cued_controlled", _) => "same_material_rehearsal_or_controlled",
                _ => "unknown",
            };
            *evidence_classes.entry(class.to_owned()).or_default() += 1;
            if let Some(outcome) = turn.get("communicative_outcome").and_then(Value::as_str) {
                *outcomes
                    .entry(class.to_owned())
                    .or_default()
                    .entry(outcome.to_owned())
                    .or_default() += 1;
            }
        }
    }
    for class in [
        "independent_first_attempt",
        "assisted_repair",
        "same_material_rehearsal_or_controlled",
        "supported_delayed_independent_transfer",
        "unknown",
    ] {
        evidence_classes.entry(class.to_owned()).or_default();
    }

    let mut valid_reviews_by_mode = BTreeMap::new();
    let mut review_stmt = c.prepare(
        "SELECT retrieval_mode,count(*) FROM weakness_reviews WHERE effective_status='active' GROUP BY retrieval_mode ORDER BY retrieval_mode",
    )?;
    for row in review_stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))? {
        let (mode, count) = row?;
        valid_reviews_by_mode.insert(mode, count);
    }
    Ok(json!({
        "scope": "all sessions through the current database boundary",
        "counts": {
            "unique_source_events": unique_source_events,
            "actual_attempts": active_attempts,
            "distinct_exercise_instances": unique_exercise_instances,
            "distinct_material_identities": unique_materials,
            "session_dates": unique_dates,
            "active_observations": active_observations,
            "disputed_observations": disputed_observations,
            "retracted_observations": retracted_observations,
            "active_findings": active_findings,
            "disputed_findings": disputed_findings,
            "active_reviews": active_reviews,
            "invalidated_reviews": invalidated_reviews
        },
        "evidence_classes": evidence_classes,
        "outcomes_by_class": outcomes,
        "last_independent_evidence_date": last_independent_date,
        "valid_reviews_by_retrieval_mode": valid_reviews_by_mode,
        "denominators": {
            "accuracy": "Only explicitly assessed, comparable opportunities qualify; incidental findings are not a denominator.",
            "independence": "Unknown assistance, exposure, cueing, or material novelty remains unknown.",
            "readiness": "No exam pass probability is inferred from these activity counts."
        },
        "status_precedence": ["assistance_overrides_transfer", "known_reused_material_is_rehearsal", "unknown_facts_remain_unknown"]
    }))
}

/// Bounded weekly exam coverage report. Official scores and model/tutor
/// estimates remain separate; absent task evidence stays unknown.
pub fn weekly_progress(c: &Connection, as_of: Option<&str>) -> Result<Value> {
    let end = as_of
        .map(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d"))
        .transpose()?
        .unwrap_or_else(|| Utc::now().with_timezone(&Madrid).date_naive());
    let start = end - chrono::Duration::days(6);
    let mut skills = serde_json::Map::new();
    for skill in ["reading", "listening", "writing", "speaking"] {
        let requirements: i64 = c.query_row(
            "SELECT count(*) FROM task_requirements WHERE source='official' AND skill=?1",
            [skill],
            |r| r.get(0),
        )?;
        let assessed_points: i64 = c.query_row(
            "SELECT count(*) FROM requested_point_assessments r JOIN attempts a ON a.id=r.attempt_id JOIN sessions s ON s.id=a.session_id WHERE s.session_date BETWEEN ?1 AND ?2 AND r.retrospective=0 AND EXISTS(SELECT 1 FROM task_requirements t WHERE t.session_id=a.session_id AND t.skill=?3 AND (t.practice_item_id=a.practice_item_id OR t.practice_item_id IS NULL)) AND (a.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=a.practice_item_id AND p.effective_status='active'))",
            params![start.to_string(), end.to_string(), skill],
            |r| r.get(0),
        )?;
        let first_attempts: i64 = c.query_row(
            "SELECT count(*) FROM attempts a JOIN sessions s ON s.id=a.session_id WHERE s.session_date BETWEEN ?1 AND ?2 AND NOT EXISTS(SELECT 1 FROM attempts earlier WHERE earlier.session_id=a.session_id AND earlier.practice_item_id IS a.practice_item_id AND earlier.attempt_no<a.attempt_no) AND EXISTS(SELECT 1 FROM task_requirements t WHERE t.session_id=a.session_id AND t.skill=?3 AND (t.practice_item_id=a.practice_item_id OR t.practice_item_id IS NULL)) AND (a.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=a.practice_item_id AND p.effective_status='active'))",
            params![start.to_string(), end.to_string(), skill],
            |r| r.get(0),
        )?;
        skills.insert(
            skill.to_owned(),
            json!({
                "status": if assessed_points > 0 { "assessments_recorded" } else if requirements > 0 { "requirements_present_no_assessments" } else { "unknown" },
                "official_task_requirements": requirements,
                "assessed_requested_points": assessed_points,
                "first_attempts_in_window": first_attempts,
                "first_attempts_basis": "First recorded attempt per task with explicit skill metadata; independence is assessed separately.",
                "denominator": "Explicit task-scoped assessments only"
            }),
        );
    }
    let timed_attempts: i64 = c.query_row(
        "SELECT count(*) FROM attempts a JOIN sessions s ON s.id=a.session_id WHERE s.session_date BETWEEN ?1 AND ?2 AND a.actual_duration_milliseconds IS NOT NULL",
        params![start.to_string(), end.to_string()],
        |r| r.get(0),
    )?;
    let duration_ms: i64 = c.query_row(
        "SELECT coalesce(sum(a.actual_duration_milliseconds),0) FROM attempts a JOIN sessions s ON s.id=a.session_id WHERE s.session_date BETWEEN ?1 AND ?2",
        params![start.to_string(), end.to_string()],
        |r| r.get(0),
    )?;
    Ok(json!({
        "window": {"start": start.to_string(), "end": end.to_string(), "timezone": "Europe/Madrid"},
        "goal": learner_goal(c)?,
        "skills": skills,
        "all_four_skills_visible": true,
        "timing": {"attempts_with_actual_duration": timed_attempts, "actual_attempt_duration_milliseconds": duration_ms, "unknown_timing_is_not_inferred": true},
        "evidence_summary": effective_evidence_summary(c)?,
        "next_priorities": crate::production::get(c)?["preferences"]["learning_focus"]["skills"],
        "readiness": "incomplete_without_compatible_component_scores"
    }))
}

pub fn word_count(text: &str) -> usize {
    text.split_whitespace()
        .filter(|token| token.chars().any(|c| c.is_alphanumeric()))
        .count()
}

pub fn word_counter_version() -> &'static str {
    "unicode-alnum-whitespace-v1"
}

fn sha256_bytes(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn json_hash(value: &Value) -> Result<String> {
    Ok(sha256_bytes(&serde_json::to_vec(value)?))
}

fn snapshot_hash(path: &Path) -> Result<String> {
    Ok(sha256_bytes(&fs::read(path).with_context(|| {
        format!("read snapshot {}", path.display())
    })?))
}

fn query_count(c: &Connection, table: &str) -> Result<i64> {
    Ok(c.query_row(
        &format!("SELECT count(*) FROM \"{}\"", table.replace('"', "\"\"")),
        [],
        |r| r.get(0),
    )?)
}

fn entity_value(c: &Connection, entity_type: &str, entity_id: &str) -> Result<Value> {
    let id = entity_id
        .parse::<i64>()
        .with_context(|| format!("{entity_type} id must be an integer"))?;
    let value = match entity_type {
        "observation" => c.query_row("SELECT o.id,o.session_id,o.attempt_id,o.practice_item_id,w.key,o.observation_no,o.outcome,o.role,o.assessment_phase,o.hint_level,o.learner_effort,o.evidence_source,o.evidence_strength,o.severity,o.produced,o.correction,o.error_span,o.notes,o.effective_status,o.revision FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.id=?1", [id], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"session_id":r.get::<_,i64>(1)?,"attempt_id":r.get::<_,Option<i64>>(2)?,"practice_item_id":r.get::<_,Option<i64>>(3)?,"weakness_key":r.get::<_,String>(4)?,"observation_no":r.get::<_,i64>(5)?,"outcome":r.get::<_,String>(6)?,"role":r.get::<_,String>(7)?,"assessment_phase":r.get::<_,String>(8)?,"hint_level":r.get::<_,String>(9)?,"learner_effort":r.get::<_,Option<String>>(10)?,"evidence_source":r.get::<_,String>(11)?,"evidence_strength":r.get::<_,Option<String>>(12)?,"severity":r.get::<_,Option<String>>(13)?,"produced":r.get::<_,Option<String>>(14)?,"correction":r.get::<_,Option<String>>(15)?,"error_span":r.get::<_,Option<String>>(16)?,"notes":r.get::<_,Option<String>>(17)?,"effective_status":r.get::<_,String>(18)?,"revision":r.get::<_,i64>(19)?})))?,
        "practice_item" => c.query_row("SELECT id,session_id,item_no,drill_type,prompt,response,corrected_response,reference_answer,feedback,outcome,item_phase,effective_status,revision,computed_word_count,word_counter_version FROM practice_items WHERE id=?1", [id], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"session_id":r.get::<_,i64>(1)?,"item_no":r.get::<_,i64>(2)?,"drill_type":r.get::<_,String>(3)?,"prompt":r.get::<_,String>(4)?,"response":r.get::<_,Option<String>>(5)?,"corrected_response":r.get::<_,Option<String>>(6)?,"reference_answer":r.get::<_,Option<String>>(7)?,"feedback":r.get::<_,Option<String>>(8)?,"outcome":r.get::<_,Option<String>>(9)?,"item_phase":r.get::<_,String>(10)?,"effective_status":r.get::<_,String>(11)?,"revision":r.get::<_,i64>(12)?,"computed_word_count":r.get::<_,Option<i64>>(13)?,"word_counter_version":r.get::<_,Option<String>>(14)?})))?,
        "finding" => c.query_row("SELECT id,session_id,practice_item_id,attempt_id,assessment_kind,original,suggestion,note,effective_status,revision FROM session_findings WHERE id=?1", [id], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"session_id":r.get::<_,i64>(1)?,"practice_item_id":r.get::<_,Option<i64>>(2)?,"attempt_id":r.get::<_,Option<i64>>(3)?,"assessment_kind":r.get::<_,String>(4)?,"original":r.get::<_,String>(5)?,"suggestion":r.get::<_,Option<String>>(6)?,"note":r.get::<_,Option<String>>(7)?,"effective_status":r.get::<_,String>(8)?,"revision":r.get::<_,i64>(9)?})))?,
        "review" => c.query_row("SELECT id,scheduler_item_id,session_id,reviewed_at,rating,retrieval_mode,evidence_strength,evidence_json,effective_status,revision FROM weakness_reviews WHERE id=?1", [id], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"scheduler_item_id":r.get::<_,i64>(1)?,"session_id":r.get::<_,i64>(2)?,"reviewed_at":r.get::<_,String>(3)?,"rating":r.get::<_,i64>(4)?,"retrieval_mode":r.get::<_,String>(5)?,"evidence_strength":r.get::<_,String>(6)?,"evidence_json":r.get::<_,String>(7)?,"effective_status":r.get::<_,String>(8)?,"revision":r.get::<_,i64>(9)?})))?,
        "session" => c.query_row("SELECT id,session_date,exercise_type_key,topic,task_ref,notes,exercise_instance_id,stimulus_identity,source_event_id,revision,(SELECT summary_json FROM effective_summaries es WHERE es.session_id=s.id AND es.summary_key='audit_quality') FROM sessions s WHERE s.id=?1", [id], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"session_date":r.get::<_,String>(1)?,"exercise_type_key":r.get::<_,String>(2)?,"topic":r.get::<_,Option<String>>(3)?,"task_ref":r.get::<_,Option<String>>(4)?,"notes":r.get::<_,Option<String>>(5)?,"exercise_instance_id":r.get::<_,Option<String>>(6)?,"stimulus_identity":r.get::<_,Option<String>>(7)?,"source_event_id":r.get::<_,Option<String>>(8)?,"revision":r.get::<_,i64>(9)?,"effective_summary":r.get::<_,Option<String>>(10)?})))?,
        _ => bail!("unsupported entity_type {entity_type}"),
    };
    Ok(value)
}

fn table_for(entity_type: &str) -> &'static str {
    match entity_type {
        "observation" => "observations",
        "practice_item" => "practice_items",
        "finding" => "session_findings",
        "review" => "weakness_reviews",
        "session" => "sessions",
        _ => "",
    }
}

fn string_field(value: &Value, field: &str) -> Option<String> {
    value.get(field).and_then(Value::as_str).map(str::to_owned)
}

fn validate_operation(p: &AuditProposal) -> Result<()> {
    if p.schema_version != AUDIT_SCHEMA_VERSION {
        bail!(
            "proposal {} uses unsupported audit schema {}",
            p.proposal_id,
            p.schema_version
        )
    }
    if p.audit_run_id.trim().is_empty()
        || p.proposal_id.trim().is_empty()
        || p.entity_id.trim().is_empty()
    {
        bail!("proposal IDs and entity_id must be non-empty")
    }
    if p.expected_revision < 1
        || p.before_hash.len() != 64
        || p.before_hash.chars().any(|c| !c.is_ascii_hexdigit())
    {
        bail!(
            "proposal {} has invalid concurrency preconditions",
            p.proposal_id
        )
    }
    if p.reason.trim().is_empty()
        || p.issue_class.trim().is_empty()
        || p.reviewer.trim().is_empty()
        || p.prompt_version.trim().is_empty()
    {
        bail!(
            "proposal {} is missing provenance or rationale",
            p.proposal_id
        )
    }
    if !p.before.is_object() || !p.after.is_object() {
        bail!(
            "proposal {} before/after values must be objects",
            p.proposal_id
        )
    }
    let allowed = |fields: &[&str]| -> Result<()> {
        for key in p.after.as_object().into_iter().flat_map(|m| m.keys()) {
            if !fields.contains(&key.as_str()) {
                bail!(
                    "proposal {} contains non-allowlisted field {key}",
                    p.proposal_id
                )
            }
        }
        Ok(())
    };
    match p.operation {
        AuditOperation::AmendAssessment => {
            if !matches!(
                p.entity_type.as_str(),
                "observation" | "practice_item" | "review"
            ) {
                bail!("amend_assessment targets an observation, practice_item, or review")
            }
            allowed(&["outcome", "effective_status"])?;
        }
        AuditOperation::RetractObservation => {
            if p.entity_type != "observation" {
                bail!("retract_observation targets an observation")
            }
            allowed(&["effective_status"])?;
            if string_field(&p.after, "effective_status").as_deref() != Some("retracted") {
                bail!("retraction must set effective_status=retracted")
            }
        }
        AuditOperation::ReclassifyFinding => {
            if p.entity_type != "finding" {
                bail!("reclassify_finding targets a finding")
            }
            allowed(&["assessment_kind", "effective_status"])?;
        }
        AuditOperation::LinkContinuation => {
            if p.entity_type != "session" {
                bail!("link_continuation targets a session")
            }
            allowed(&[
                "exercise_instance_id",
                "stimulus_identity",
                "source_event_id",
            ])?;
        }
        AuditOperation::MarkDuplicate => {
            if !matches!(
                p.entity_type.as_str(),
                "observation" | "practice_item" | "finding"
            ) {
                bail!("mark_duplicate targets an observation, practice_item, or finding")
            }
            allowed(&["effective_status"])?;
            if string_field(&p.after, "effective_status").as_deref() != Some("retracted") {
                bail!("duplicate marking must set effective_status=retracted")
            }
        }
        AuditOperation::ReassignTarget => {
            if p.entity_type != "observation" {
                bail!("reassign_target targets an observation")
            }
            allowed(&["weakness_key"])?;
            if string_field(&p.after, "weakness_key").is_none() {
                bail!("reassign_target requires after.weakness_key")
            }
        }
        AuditOperation::AddRecoveredFinding => {
            if p.entity_type != "session" {
                bail!("add_recovered_finding uses the containing session as entity")
            }
            for key in [
                "practice_item_id",
                "attempt_id",
                "assessment_kind",
                "original",
                "suggestion",
                "note",
            ] {
                if key != "suggestion" && key != "note" && p.after.get(key).is_none() {
                    bail!("recovered finding requires after.{key}")
                }
            }
        }
        AuditOperation::InvalidateReview => {
            if p.entity_type != "review" {
                bail!("invalidate_review targets a review")
            }
            allowed(&["effective_status"])?;
            if string_field(&p.after, "effective_status").as_deref() != Some("invalidated") {
                bail!("review invalidation must set effective_status=invalidated")
            }
        }
        AuditOperation::AmendSummary => {
            if p.entity_type != "session" {
                bail!("amend_summary uses a session anchor")
            }
            allowed(&["summary_key", "summary_value"])?;
            if p.after.get("summary_key").and_then(Value::as_str).is_none()
                || p.after.get("summary_value").is_none()
            {
                bail!("amend_summary requires summary_key and summary_value")
            }
        }
    }
    Ok(())
}

fn validate_source_refs(c: &Connection, p: &AuditProposal) -> Result<()> {
    for source in &p.source_refs {
        if source.source_id.trim().is_empty() {
            bail!("proposal {} has an empty source reference", p.proposal_id)
        }
        if let Some(hash) = source.source_hash.as_deref()
            && (hash.len() != 64 || hash.chars().any(|c| !c.is_ascii_hexdigit()))
        {
            bail!("proposal {} has an invalid source hash", p.proposal_id)
        }
        if source.source_id.starts_with("amendment:") && source.excerpt.is_none() {
            continue;
        }
        let (source_type, source_id, field) = source
            .source_id
            .split_once(':')
            .and_then(|(kind, rest)| rest.split_once(':').map(|(id, field)| (kind, id, field)))
            .ok_or_else(|| {
                anyhow!(
                    "source reference {} must use entity:id:field",
                    source.source_id
                )
            })?;
        let current = entity_value(c, source_type, source_id)?;
        if let Some(excerpt) = source.excerpt.as_deref().filter(|x| !x.is_empty()) {
            let text = current
                .get(field)
                .and_then(Value::as_str)
                .ok_or_else(|| anyhow!("source reference field {field} is not textual"))?;
            if !text.contains(excerpt) {
                bail!(
                    "proposal {} source excerpt does not match {}",
                    p.proposal_id,
                    source.source_id
                )
            }
        }
    }
    Ok(())
}

pub fn read_batch(path: &Path) -> Result<AuditBatch> {
    let bytes = fs::read(path).with_context(|| format!("read audit batch {}", path.display()))?;
    serde_json::from_slice(&bytes).with_context(|| format!("parse audit batch {}", path.display()))
}

pub fn validate_batch(c: &Connection, batch: &AuditBatch) -> Result<()> {
    if batch.audit_run_id.trim().is_empty()
        || batch.snapshot_boundary.trim().is_empty()
        || batch.snapshot_hash.len() != 64
        || batch.prompt_version.trim().is_empty()
    {
        bail!("audit batch metadata is incomplete")
    }
    let mut proposal_ids = HashSet::new();
    for p in &batch.proposals {
        if p.audit_run_id != batch.audit_run_id {
            bail!(
                "proposal {} belongs to a different audit run",
                p.proposal_id
            )
        }
        if !proposal_ids.insert(p.proposal_id.clone()) {
            bail!("duplicate proposal_id {}", p.proposal_id)
        }
        validate_operation(p)?;
        let already_applied: Option<(String, String, String)> = c
            .query_row(
                "SELECT status,operation,after_json FROM data_amendments WHERE audit_run_id=?1 AND proposal_id=?2",
                params![p.audit_run_id, p.proposal_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        if let Some((status, operation, after_json)) = already_applied {
            if status != "applied"
                || operation != p.operation.as_str()
                || after_json != p.after.to_string()
            {
                bail!(
                    "proposal {} conflicts with its recorded amendment",
                    p.proposal_id
                )
            }
            continue;
        }
        if table_for(&p.entity_type).is_empty()
            && !matches!(p.operation, AuditOperation::AddRecoveredFinding)
        {
            bail!("unsupported entity type {}", p.entity_type)
        }
        let current = if matches!(p.operation, AuditOperation::AddRecoveredFinding) {
            entity_value(c, "session", &p.entity_id)?
        } else {
            entity_value(c, &p.entity_type, &p.entity_id)?
        };
        if p.source_refs.is_empty() && p.certainty == Certainty::Supported {
            bail!("supported proposal {} must cite source_refs", p.proposal_id)
        }
        validate_source_refs(c, p)?;
        if current.get("revision").and_then(Value::as_i64) != Some(p.expected_revision)
            && !matches!(p.operation, AuditOperation::AddRecoveredFinding)
        {
            bail!(
                "proposal {} expected revision {} but current entity is {:?}",
                p.proposal_id,
                p.expected_revision,
                current.get("revision")
            )
        }
        if !matches!(p.operation, AuditOperation::AddRecoveredFinding)
            && json_hash(&current)? != p.before_hash
        {
            bail!(
                "proposal {} before_hash does not match current entity",
                p.proposal_id
            )
        }
        for dependency in &p.dependencies {
            if !proposal_ids.contains(dependency)
                && !batch
                    .proposals
                    .iter()
                    .any(|x| format!("{}:{}", x.audit_run_id, x.proposal_id) == *dependency)
            {
                bail!(
                    "proposal {} references missing dependency {}",
                    p.proposal_id,
                    dependency
                )
            }
        }
    }
    ensure_acyclic(batch)?;
    Ok(())
}

fn ensure_acyclic(batch: &AuditBatch) -> Result<()> {
    let keys = batch
        .proposals
        .iter()
        .map(|p| p.proposal_id.clone())
        .collect::<HashSet<_>>();
    let mut graph = HashMap::<String, Vec<String>>::new();
    for p in &batch.proposals {
        let dependencies = p
            .dependencies
            .iter()
            .map(|d| {
                d.strip_prefix(&format!("{}:", batch.audit_run_id))
                    .unwrap_or(d)
                    .to_owned()
            })
            .filter(|d| keys.contains(d))
            .collect();
        graph.insert(p.proposal_id.clone(), dependencies);
    }
    fn visit(
        node: &str,
        graph: &HashMap<String, Vec<String>>,
        marks: &mut HashMap<String, u8>,
    ) -> Result<()> {
        if marks.get(node) == Some(&1) {
            bail!("audit proposal dependency cycle at {node}")
        }
        if marks.get(node) == Some(&2) {
            return Ok(());
        }
        marks.insert(node.to_owned(), 1);
        for next in graph.get(node).into_iter().flatten() {
            visit(next, graph, marks)?;
        }
        marks.insert(node.to_owned(), 2);
        Ok(())
    }
    let mut marks = HashMap::new();
    for key in keys {
        visit(&key, &graph, &mut marks)?;
    }
    Ok(())
}

fn set_status(tx: &Transaction<'_>, p: &AuditProposal) -> Result<()> {
    let id = p.entity_id.parse::<i64>()?;
    let status = string_field(&p.after, "effective_status");
    match p.operation {
        AuditOperation::AmendAssessment => {
            let table = match p.entity_type.as_str() {
                "observation" => "observations",
                "practice_item" => "practice_items",
                "review" => "weakness_reviews",
                _ => bail!("amend_assessment targets an assessment entity"),
            };
            let outcome = p.after.get("outcome").and_then(Value::as_str);
            if table == "observations" {
                tx.execute("UPDATE observations SET outcome=COALESCE(?1,outcome),effective_status=COALESCE(?2,effective_status),revision=revision+1 WHERE id=?3 AND revision=?4", params![outcome,status,id,p.expected_revision])?;
            } else if table == "weakness_reviews" {
                tx.execute("UPDATE weakness_reviews SET effective_status=COALESCE(?1,effective_status),revision=revision+1 WHERE id=?2 AND revision=?3", params![status,id,p.expected_revision])?;
            } else {
                tx.execute("UPDATE practice_items SET outcome=COALESCE(?1,outcome),effective_status=COALESCE(?2,effective_status),revision=revision+1 WHERE id=?3 AND revision=?4", params![outcome,status,id,p.expected_revision])?;
            }
        }
        AuditOperation::RetractObservation | AuditOperation::MarkDuplicate => {
            let table = match p.entity_type.as_str() {
                "observation" => "observations",
                "practice_item" => "practice_items",
                "finding" => "session_findings",
                _ => bail!("invalid duplicate target"),
            };
            tx.execute(&format!("UPDATE {table} SET effective_status='retracted',revision=revision+1 WHERE id=?1 AND revision=?2"), params![id,p.expected_revision])?;
        }
        AuditOperation::ReclassifyFinding => {
            tx.execute("UPDATE session_findings SET assessment_kind=COALESCE(?1,assessment_kind),effective_status=COALESCE(?2,effective_status),revision=revision+1 WHERE id=?3 AND revision=?4", params![p.after.get("assessment_kind").and_then(Value::as_str),status,id,p.expected_revision])?;
        }
        AuditOperation::LinkContinuation => {
            tx.execute("UPDATE sessions SET exercise_instance_id=?1,stimulus_identity=?2,source_event_id=?3,revision=revision+1 WHERE id=?4 AND revision=?5", params![p.after.get("exercise_instance_id").and_then(Value::as_str),p.after.get("stimulus_identity").and_then(Value::as_str),p.after.get("source_event_id").and_then(Value::as_str),id,p.expected_revision])?;
        }
        AuditOperation::ReassignTarget => {
            tx.execute("UPDATE observations SET weakness_id=(SELECT id FROM weaknesses WHERE key=?1),revision=revision+1 WHERE id=?2 AND revision=?3 AND EXISTS(SELECT 1 FROM weaknesses WHERE key=?1)", params![p.after.get("weakness_key").and_then(Value::as_str),id,p.expected_revision])?;
        }
        AuditOperation::InvalidateReview => {
            tx.execute("UPDATE weakness_reviews SET effective_status='invalidated',revision=revision+1 WHERE id=?1 AND revision=?2", params![id,p.expected_revision])?;
        }
        AuditOperation::AddRecoveredFinding => {
            let session_id = id;
            tx.execute("INSERT INTO session_findings(session_id,practice_item_id,attempt_id,assessment_kind,original,suggestion,note) VALUES(?1,?2,?3,?4,?5,?6,?7)", params![session_id,p.after.get("practice_item_id").and_then(Value::as_i64),p.after.get("attempt_id").and_then(Value::as_i64),p.after.get("assessment_kind").and_then(Value::as_str),p.after.get("original").and_then(Value::as_str),p.after.get("suggestion").and_then(Value::as_str),p.after.get("note").and_then(Value::as_str)])?;
        }
        AuditOperation::AmendSummary => {
            tx.execute(
                "UPDATE sessions SET revision=revision+1 WHERE id=?1 AND revision=?2",
                params![id, p.expected_revision],
            )?;
            if p.after.get("summary_value").is_some_and(Value::is_null) {
                tx.execute(
                    "DELETE FROM effective_summaries WHERE session_id=?1 AND summary_key=?2",
                    params![id, p.after.get("summary_key").and_then(Value::as_str)],
                )?;
            } else {
                tx.execute("INSERT INTO effective_summaries(session_id,summary_key,summary_json,revision,updated_at) VALUES(?1,?2,?3,?4,?5) ON CONFLICT(session_id,summary_key) DO UPDATE SET summary_json=excluded.summary_json,revision=excluded.revision,updated_at=excluded.updated_at", params![id,p.after.get("summary_key").and_then(Value::as_str),p.after.get("summary_value").unwrap_or(&Value::Null).to_string(),p.expected_revision+1,Utc::now().to_rfc3339_opts(SecondsFormat::Millis,true)])?;
            }
        }
    }
    if !matches!(p.operation, AuditOperation::AddRecoveredFinding) {
        let expected = p.expected_revision + 1;
        let current: i64 = tx
            .query_row(
                &format!(
                    "SELECT revision FROM {} WHERE id=?1",
                    table_for(&p.entity_type)
                ),
                [id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| {
                anyhow!(
                    "entity {}:{} disappeared while applying",
                    p.entity_type,
                    p.entity_id
                )
            })?;
        if current != expected {
            bail!(
                "proposal {} affected zero or multiple stale rows",
                p.proposal_id
            )
        }
    }
    Ok(())
}

fn apply_batch_tx(tx: &Transaction<'_>, batch: &AuditBatch) -> Result<(i64, i64)> {
    tx.execute("INSERT OR IGNORE INTO audit_runs(audit_run_id,snapshot_boundary,snapshot_hash,model_id,prompt_version,scope_json,started_at,status,proposed_count) VALUES(?1,?2,?3,?4,?5,?6,?7,'running',?8)", params![batch.audit_run_id,batch.snapshot_boundary,batch.snapshot_hash,batch.model_id,batch.prompt_version,batch.scope.to_string(),Utc::now().to_rfc3339_opts(SecondsFormat::Millis,true),i64::try_from(batch.proposals.len())?])?;
    let mut applied = 0i64;
    let mut skipped = 0i64;
    for p in &batch.proposals {
        let existing: Option<String> = tx
            .query_row(
                "SELECT status FROM data_amendments WHERE audit_run_id=?1 AND proposal_id=?2",
                params![p.audit_run_id, p.proposal_id],
                |r| r.get(0),
            )
            .optional()?;
        if existing.as_deref() == Some("applied") {
            skipped += 1;
            continue;
        }
        let current = if matches!(p.operation, AuditOperation::AddRecoveredFinding) {
            entity_value(tx, "session", &p.entity_id)?
        } else {
            entity_value(tx, &p.entity_type, &p.entity_id)?
        };
        if !matches!(p.operation, AuditOperation::AddRecoveredFinding)
            && (current["revision"] != json!(p.expected_revision)
                || json_hash(&current)? != p.before_hash)
        {
            bail!(
                "proposal {} is stale; current revision/hash changed",
                p.proposal_id
            )
        }
        set_status(tx, p)?;
        tx.execute("INSERT INTO data_amendments(audit_run_id,proposal_id,entity_type,entity_id,expected_revision,before_hash,operation,before_json,after_json,source_refs_json,reason,issue_class,certainty,reviewer,prompt_version,reviewed_at,dependencies_json,affected_consumers_json,predecessor_revision,status,applied_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,'applied',?20)", params![p.audit_run_id,p.proposal_id,p.entity_type,p.entity_id,p.expected_revision,p.before_hash,p.operation.as_str(),p.before.to_string(),p.after.to_string(),serde_json::to_string(&p.source_refs)?,p.reason,p.issue_class,p.certainty.as_str(),p.reviewer,p.prompt_version,p.reviewed_at,serde_json::to_string(&p.dependencies)?,serde_json::to_string(&p.affected_consumers)?,p.expected_revision,Utc::now().to_rfc3339_opts(SecondsFormat::Millis,true)])?;
        applied += 1;
    }
    tx.execute("UPDATE audit_runs SET status='completed',ended_at=strftime('%Y-%m-%dT%H:%M:%fZ','now'),applied_count=applied_count+?2,latest_successful_boundary=?3 WHERE audit_run_id=?1", params![batch.audit_run_id,applied,batch.snapshot_boundary])?;
    Ok((applied, skipped))
}

pub fn apply_batch(c: &mut Connection, batch: &AuditBatch) -> Result<(i64, i64)> {
    validate_batch(c, batch)?;
    let tx = c.transaction()?;
    let result = apply_batch_tx(&tx, batch);
    match result {
        Ok(value) => {
            tx.commit()?;
            Ok(value)
        }
        Err(error) => {
            let _ = tx.rollback();
            Err(error)
        }
    }
}

pub fn preview_batch(c: &Connection, batch: &AuditBatch) -> Result<Value> {
    validate_batch(c, batch)?;
    let mut changes = Vec::new();
    for p in &batch.proposals {
        let current = if matches!(p.operation, AuditOperation::AddRecoveredFinding) {
            entity_value(c, "session", &p.entity_id)?
        } else {
            entity_value(c, &p.entity_type, &p.entity_id)?
        };
        changes.push(json!({"proposal_id":p.proposal_id,"operation":p.operation,"entity_type":p.entity_type,"entity_id":p.entity_id,"before":current,"after":p.after,"certainty":p.certainty,"affected_consumers":p.affected_consumers}));
    }
    Ok(
        json!({"audit_run_id":batch.audit_run_id,"snapshot_boundary":batch.snapshot_boundary,"proposal_count":batch.proposals.len(),"changes":changes,"unresolved_cases":batch.proposals.iter().filter(|p|p.certainty != Certainty::Supported).map(|p|p.proposal_id.clone()).collect::<Vec<_>>(),"scheduler_impact":"Affected scheduler tracks are flagged by consumer metadata; no replacement rating is invented."}),
    )
}

pub fn verify_database(c: &Connection, batch_id: Option<&str>) -> Result<Value> {
    let integrity: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if integrity != "ok" {
        bail!("SQLite integrity_check failed: {integrity}")
    }
    if c.prepare("PRAGMA foreign_key_check")?
        .query([])?
        .next()?
        .is_some()
    {
        bail!("SQLite foreign_key_check failed")
    }
    let mut counts = BTreeMap::new();
    for table in [
        "sessions",
        "attempts",
        "practice_items",
        "observations",
        "session_findings",
        "weaknesses",
        "weakness_reviews",
        "recorded_requests",
        "audit_runs",
        "data_amendments",
        "effective_summaries",
    ] {
        counts.insert(table.to_owned(), query_count(c, table)?);
    }
    let effective: BTreeMap<String, i64> = [
        (
            "active_observations",
            c.query_row(
                "SELECT count(*) FROM observations WHERE effective_status='active'",
                [],
                |r| r.get(0),
            )?,
        ),
        (
            "retracted_observations",
            c.query_row(
                "SELECT count(*) FROM observations WHERE effective_status='retracted'",
                [],
                |r| r.get(0),
            )?,
        ),
        (
            "active_findings",
            c.query_row(
                "SELECT count(*) FROM session_findings WHERE effective_status='active'",
                [],
                |r| r.get(0),
            )?,
        ),
        (
            "invalidated_reviews",
            c.query_row(
                "SELECT count(*) FROM weakness_reviews WHERE effective_status='invalidated'",
                [],
                |r| r.get(0),
            )?,
        ),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), v))
    .collect();
    Ok(
        json!({"integrity":"ok","foreign_keys":"ok","counts":counts,"effective":effective,"batch_id":batch_id,"amendments_for_batch":batch_id.and_then(|id|c.query_row("SELECT count(*) FROM data_amendments WHERE audit_run_id=?1",[id],|r|r.get::<_,i64>(0)).ok())}),
    )
}

pub fn revert_batch(
    c: &mut Connection,
    batch_id: &str,
    snapshot_hash: &str,
) -> Result<(String, i64)> {
    if batch_id.trim().is_empty() {
        bail!("batch id must be non-empty")
    }
    let rows = {
        let mut stmt = c.prepare("SELECT proposal_id,entity_type,entity_id,expected_revision,before_hash,operation,before_json,after_json,source_refs_json,reason,issue_class,reviewer,prompt_version,reviewed_at,dependencies_json,affected_consumers_json FROM data_amendments WHERE audit_run_id=?1 AND status='applied' ORDER BY rowid DESC")?;
        stmt.query_map([batch_id], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, String>(6)?,
                r.get::<_, String>(7)?,
                r.get::<_, String>(8)?,
                r.get::<_, String>(9)?,
                r.get::<_, String>(10)?,
                r.get::<_, String>(11)?,
                r.get::<_, String>(12)?,
                r.get::<_, String>(13)?,
                r.get::<_, String>(14)?,
                r.get::<_, String>(15)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?
    };
    if rows.is_empty() {
        bail!("no applied amendments found for audit run {batch_id}")
    }
    let new_id = format!("{batch_id}-revert-{}", Utc::now().format("%Y%m%dT%H%M%SZ"));
    let proposals = rows.into_iter().enumerate().map(|(index,(proposal_id,entity_type,entity_id,_expected,_old_hash,operation,before_json,after_json,source_refs,reason,issue_class,reviewer,prompt_version,reviewed_at,_dependencies,affected))| -> Result<AuditProposal> {
        let current = entity_value(c, &entity_type, &entity_id)?;
        let before: Value = serde_json::from_str(&after_json)?;
        let original: Value = serde_json::from_str(&before_json)?;
        let op = match operation.as_str() {
            "retract_observation" | "mark_duplicate" => AuditOperation::AmendAssessment,
            "reclassify_finding" => AuditOperation::ReclassifyFinding,
            "link_continuation" => AuditOperation::LinkContinuation,
            "reassign_target" => AuditOperation::ReassignTarget,
            "invalidate_review" => AuditOperation::AmendAssessment,
            "amend_assessment" => AuditOperation::AmendAssessment,
            "add_recovered_finding" => AuditOperation::MarkDuplicate,
            _ => AuditOperation::AmendSummary,
        };
        let after = match op {
            AuditOperation::AmendAssessment => { let mut v = json!({}); if let Some(x)=original.get("outcome"){v["outcome"]=x.clone();} if let Some(x)=original.get("effective_status"){v["effective_status"]=x.clone();} v },
            AuditOperation::ReclassifyFinding => { let mut v=json!({}); if let Some(x)=original.get("assessment_kind"){v["assessment_kind"]=x.clone();} if let Some(x)=original.get("effective_status"){v["effective_status"]=x.clone();} v },
            AuditOperation::LinkContinuation => { let mut v=json!({}); for k in ["exercise_instance_id","stimulus_identity","source_event_id"] {if let Some(x)=original.get(k){v[k]=x.clone();} else {v[k]=Value::Null;}} v },
            AuditOperation::ReassignTarget => json!({"weakness_key":original.get("weakness_key").cloned().unwrap_or(Value::Null)}),
            AuditOperation::MarkDuplicate => json!({"effective_status":"retracted"}),
            AuditOperation::AmendSummary => {
                let mut value = json!({});
                value["summary_key"] = json!("audit_quality");
                value["summary_value"] = original.get("effective_summary").cloned().unwrap_or(Value::Null);
                value
            }
            _ => json!({}),
        };
        let mut source = serde_json::from_str::<Vec<SourceRef>>(&source_refs)?;
        source.push(SourceRef{source_id:format!("amendment:{batch_id}:{proposal_id}"),excerpt:None,span:None,source_hash:Some(snapshot_hash.to_owned())});
        Ok(AuditProposal{audit_run_id:new_id.clone(),proposal_id:format!("revert-{index}-{}",proposal_id),schema_version:AUDIT_SCHEMA_VERSION,entity_type,entity_id,expected_revision:current["revision"].as_i64().unwrap_or(1),before_hash:json_hash(&current)?,operation:op,before,after,source_refs:source,reason:format!("Compensating amendment for {batch_id}: {reason}"),issue_class,certainty:Certainty::Supported,reviewer, prompt_version,reviewed_at,dependencies:Vec::new(),affected_consumers:serde_json::from_str(&affected)?})
    }).collect::<Result<Vec<_>>>()?;
    let batch = AuditBatch {
        audit_run_id: new_id.clone(),
        snapshot_boundary: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        snapshot_hash: snapshot_hash.to_owned(),
        model_id: None,
        prompt_version: "compensating-revert-v1".to_owned(),
        scope: json!({"compensates":batch_id}),
        proposals,
    };
    let count = apply_batch(c, &batch)?.0;
    Ok((new_id, count))
}

fn session_export(c: &Connection, sid: i64) -> Result<Value> {
    let session = entity_value(c, "session", &sid.to_string())?;
    let mut out = session;
    let items = c.prepare("SELECT id,item_no,drill_type,prompt,prompt_fingerprint,response,corrected_response,reference_answer,feedback,outcome,activity_run_id,item_phase,created_at,effective_status,revision,computed_word_count,word_counter_version FROM practice_items WHERE session_id=?1 ORDER BY item_no")?.query_map([sid], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"item_no":r.get::<_,i64>(1)?,"drill_type":r.get::<_,String>(2)?,"prompt":r.get::<_,String>(3)?,"prompt_fingerprint":r.get::<_,Option<String>>(4)?,"response":r.get::<_,Option<String>>(5)?,"corrected_response":r.get::<_,Option<String>>(6)?,"reference_answer":r.get::<_,Option<String>>(7)?,"feedback":r.get::<_,Option<String>>(8)?,"outcome":r.get::<_,Option<String>>(9)?,"activity_run_id":r.get::<_,Option<i64>>(10)?,"item_phase":r.get::<_,String>(11)?,"created_at":r.get::<_,String>(12)?,"effective_status":r.get::<_,String>(13)?,"revision":r.get::<_,i64>(14)?,"computed_word_count":r.get::<_,Option<i64>>(15)?,"word_counter_version":r.get::<_,Option<String>>(16)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let attempts = c.prepare("SELECT id,attempt_no,practice_item_id,transcript,target_duration_seconds,actual_duration_milliseconds,response_mode,response_latency_milliseconds,timing_source,created_at FROM attempts WHERE session_id=?1 ORDER BY attempt_no")?.query_map([sid], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"attempt_no":r.get::<_,i64>(1)?,"practice_item_id":r.get::<_,Option<i64>>(2)?,"transcript":r.get::<_,String>(3)?,"target_duration_seconds":r.get::<_,Option<i64>>(4)?,"actual_duration_milliseconds":r.get::<_,Option<i64>>(5)?,"response_mode":r.get::<_,Option<String>>(6)?,"response_latency_milliseconds":r.get::<_,Option<i64>>(7)?,"timing_source":r.get::<_,Option<String>>(8)?,"created_at":r.get::<_,String>(9)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let observations = c.prepare("SELECT o.id,o.attempt_id,o.practice_item_id,w.key,o.observation_no,o.outcome,o.role,o.assessment_phase,o.hint_level,o.learner_effort,o.evidence_source,o.evidence_strength,o.severity,o.produced,o.correction,o.error_span,o.notes,o.created_at,o.effective_status,o.revision FROM observations o JOIN weaknesses w ON w.id=o.weakness_id WHERE o.session_id=?1 ORDER BY o.observation_no")?.query_map([sid], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"attempt_id":r.get::<_,Option<i64>>(1)?,"practice_item_id":r.get::<_,Option<i64>>(2)?,"weakness_key":r.get::<_,String>(3)?,"observation_no":r.get::<_,i64>(4)?,"outcome":r.get::<_,String>(5)?,"role":r.get::<_,String>(6)?,"assessment_phase":r.get::<_,String>(7)?,"hint_level":r.get::<_,String>(8)?,"learner_effort":r.get::<_,Option<String>>(9)?,"evidence_source":r.get::<_,String>(10)?,"evidence_strength":r.get::<_,Option<String>>(11)?,"severity":r.get::<_,Option<String>>(12)?,"produced":r.get::<_,Option<String>>(13)?,"correction":r.get::<_,Option<String>>(14)?,"error_span":r.get::<_,Option<String>>(15)?,"notes":r.get::<_,Option<String>>(16)?,"created_at":r.get::<_,String>(17)?,"effective_status":r.get::<_,String>(18)?,"revision":r.get::<_,i64>(19)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let findings = c.prepare("SELECT id,practice_item_id,attempt_id,assessment_kind,original,suggestion,note,created_at,effective_status,revision FROM session_findings WHERE session_id=?1 ORDER BY id")?.query_map([sid], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"practice_item_id":r.get::<_,Option<i64>>(1)?,"attempt_id":r.get::<_,Option<i64>>(2)?,"assessment_kind":r.get::<_,String>(3)?,"original":r.get::<_,String>(4)?,"suggestion":r.get::<_,Option<String>>(5)?,"note":r.get::<_,Option<String>>(6)?,"created_at":r.get::<_,String>(7)?,"effective_status":r.get::<_,String>(8)?,"revision":r.get::<_,i64>(9)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let reviews = c.prepare("SELECT id,scheduler_item_id,reviewed_at,rating,rating_source,rating_rationale,retrieval_mode,evidence_strength,evidence_json,elapsed_days,desired_retention,algorithm,algorithm_version,parameter_set_id,effective_status,revision FROM weakness_reviews WHERE session_id=?1 ORDER BY id")?.query_map([sid], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"scheduler_item_id":r.get::<_,i64>(1)?,"reviewed_at":r.get::<_,String>(2)?,"rating":r.get::<_,i64>(3)?,"rating_source":r.get::<_,String>(4)?,"rating_rationale":r.get::<_,Option<String>>(5)?,"retrieval_mode":r.get::<_,String>(6)?,"evidence_strength":r.get::<_,String>(7)?,"evidence_json":r.get::<_,String>(8)?,"elapsed_days":r.get::<_,i64>(9)?,"desired_retention":r.get::<_,f64>(10)?,"algorithm":r.get::<_,String>(11)?,"algorithm_version":r.get::<_,String>(12)?,"parameter_set_id":r.get::<_,i64>(13)?,"effective_status":r.get::<_,String>(14)?,"revision":r.get::<_,i64>(15)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let task_requirements = c
        .prepare("SELECT id,practice_item_id,exam_key,specification_version,skill,task_key,prompt_hash,requested_points_json,word_min,word_max,time_budget_seconds,preparation_budget_seconds,source,created_at FROM task_requirements WHERE session_id=?1 ORDER BY id")?
        .query_map([sid], |r| {
            let requested_points: Value = serde_json::from_str(&r.get::<_, String>(7)?)
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            Ok(json!({"id":r.get::<_,i64>(0)?,"practice_item_id":r.get::<_,Option<i64>>(1)?,"exam_key":r.get::<_,Option<String>>(2)?,"specification_version":r.get::<_,Option<String>>(3)?,"skill":r.get::<_,String>(4)?,"task_key":r.get::<_,String>(5)?,"prompt_hash":r.get::<_,Option<String>>(6)?,"requested_points":requested_points,"word_min":r.get::<_,Option<i64>>(8)?,"word_max":r.get::<_,Option<i64>>(9)?,"time_budget_seconds":r.get::<_,Option<i64>>(10)?,"preparation_budget_seconds":r.get::<_,Option<i64>>(11)?,"source":r.get::<_,String>(12)?,"created_at":r.get::<_,String>(13)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let point_assessments = c
        .prepare("SELECT r.id,r.attempt_id,a.attempt_no,r.point_key,r.status,r.supporting_excerpt,r.reason,r.evaluator,r.rubric_version,r.assessed_at,r.retrospective FROM requested_point_assessments r JOIN attempts a ON a.id=r.attempt_id WHERE a.session_id=?1 ORDER BY r.id")?
        .query_map([sid], |r| {
            Ok(json!({"id":r.get::<_,i64>(0)?,"attempt_id":r.get::<_,i64>(1)?,"attempt_no":r.get::<_,i64>(2)?,"point_key":r.get::<_,String>(3)?,"status":r.get::<_,String>(4)?,"supporting_excerpt":r.get::<_,Option<String>>(5)?,"reason":r.get::<_,Option<String>>(6)?,"evaluator":r.get::<_,String>(7)?,"rubric_version":r.get::<_,Option<String>>(8)?,"assessed_at":r.get::<_,String>(9)?,"retrospective":r.get::<_,bool>(10)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    out["items"] = json!(items);
    out["attempts"] = json!(attempts);
    out["observations"] = json!(observations);
    out["findings"] = json!(findings);
    out["reviews"] = json!(reviews);
    out["task_requirements"] = json!(task_requirements);
    out["requested_point_assessments"] = json!(point_assessments);
    out["production"] = crate::production::session_audit(c, sid)?;
    Ok(out)
}

fn catalog_export(c: &Connection) -> Result<Value> {
    let weaknesses = c
        .prepare("SELECT id,key,category,description,target_pattern,recommended_drill_types,first_seen,last_seen,target_type,target_status,classification_pending,active FROM weaknesses ORDER BY key")?
        .query_map([], |r| {
            let drills = r
                .get::<_, Option<String>>(5)?
                .map(|value| serde_json::from_str::<Value>(&value))
                .transpose()
                .map_err(|error| rusqlite::Error::ToSqlConversionFailure(Box::new(error)))?;
            Ok(json!({"id":r.get::<_,i64>(0)?,"key":r.get::<_,String>(1)?,"category":r.get::<_,String>(2)?,"description":r.get::<_,String>(3)?,"target_pattern":r.get::<_,Option<String>>(4)?,"recommended_drill_types":drills,"first_seen":r.get::<_,Option<String>>(6)?,"last_seen":r.get::<_,Option<String>>(7)?,"target_type":r.get::<_,String>(8)?,"target_status":r.get::<_,String>(9)?,"classification_pending":r.get::<_,bool>(10)?,"active":r.get::<_,bool>(11)?}))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let concepts = c
        .prepare("SELECT c.key,c.label,cs.key,c.definition,c.source_uri,c.active FROM concepts c JOIN concept_schemes cs ON cs.id=c.scheme_id ORDER BY c.key")?
        .query_map([], |r| Ok(json!({"key":r.get::<_,String>(0)?,"label":r.get::<_,String>(1)?,"scheme_key":r.get::<_,String>(2)?,"definition":r.get::<_,Option<String>>(3)?,"source_uri":r.get::<_,Option<String>>(4)?,"active":r.get::<_,bool>(5)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let edges = c
        .prepare("SELECT a.key,e.predicate,b.key,e.provenance,e.notes FROM concept_edges e JOIN concepts a ON a.id=e.subject_id JOIN concepts b ON b.id=e.object_id ORDER BY a.key,e.predicate,b.key")?
        .query_map([], |r| Ok(json!({"subject":r.get::<_,String>(0)?,"predicate":r.get::<_,String>(1)?,"object":r.get::<_,String>(2)?,"provenance":r.get::<_,String>(3)?,"notes":r.get::<_,Option<String>>(4)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let stimuli = c
        .prepare("SELECT s.id,r.session_id,r.run_no,s.stimulus_no,s.kind,s.delivery_mode,s.content_text,s.source_uri,s.content_fingerprint,s.created_at FROM activity_stimuli s JOIN activity_runs r ON r.id=s.activity_run_id ORDER BY r.session_id,r.run_no,s.stimulus_no")?
        .query_map([], |r| Ok(json!({"id":r.get::<_,i64>(0)?,"session_id":r.get::<_,i64>(1)?,"run_no":r.get::<_,i64>(2)?,"stimulus_no":r.get::<_,i64>(3)?,"kind":r.get::<_,String>(4)?,"delivery_mode":r.get::<_,String>(5)?,"content_text":r.get::<_,Option<String>>(6)?,"source_uri":r.get::<_,Option<String>>(7)?,"content_fingerprint":r.get::<_,Option<String>>(8)?,"created_at":r.get::<_,String>(9)?})))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut scheduler_stmt = c.prepare("SELECT p.algorithm,p.algorithm_version,p.parameters_json,p.source,c.desired_retention,c.timezone,c.day_cutoff_hour FROM scheduler_config c JOIN scheduler_parameter_sets p ON p.id=c.active_parameter_set_id")?;
    let scheduler_row: (String, String, String, String, f64, String, i64) = scheduler_stmt
        .query_row([], |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
                r.get(6)?,
            ))
        })?;
    let scheduler = json!({"algorithm":scheduler_row.0,"algorithm_version":scheduler_row.1,"parameters":serde_json::from_str::<Value>(&scheduler_row.2)?,"source":scheduler_row.3,"desired_retention":scheduler_row.4,"timezone":scheduler_row.5,"day_cutoff_hour":scheduler_row.6});
    Ok(
        json!({"weaknesses":weaknesses,"concepts":concepts,"concept_edges":edges,"activity_stimuli":stimuli,"practice_preferences":crate::production::get(c)?,"scheduler":scheduler}),
    )
}

pub fn export_database(snapshot: &Path, output_dir: &Path) -> Result<ExportManifest> {
    fs::create_dir_all(output_dir)?;
    let _ = fs::set_permissions(output_dir, fs::Permissions::from_mode(0o700));
    let c = Connection::open_with_flags(snapshot, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let hash = snapshot_hash(snapshot)?;
    let boundary: String = c.query_row(
        "SELECT COALESCE(max(created_at),'') FROM sessions",
        [],
        |r| r.get(0),
    )?;
    let schema_version: String = c.query_row(
        "SELECT value FROM schema_meta WHERE key='schema_version'",
        [],
        |r| r.get(0),
    )?;
    let table_names = [
        "sessions",
        "attempts",
        "practice_items",
        "observations",
        "session_findings",
        "weaknesses",
        "weakness_reviews",
        "review_observations",
        "recorded_requests",
    ];
    let mut counts = BTreeMap::new();
    for table in table_names {
        counts.insert(table.to_owned(), query_count(&c, table)?);
    }
    let session_ids: Vec<i64> = c
        .prepare("SELECT id FROM sessions WHERE created_at<=?1 ORDER BY created_at,id")?
        .query_map([&boundary], |r| r.get(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let sessions = session_ids
        .iter()
        .map(|id| session_export(&c, *id))
        .collect::<Result<Vec<_>>>()?;
    let progress_as_of = boundary
        .get(..10)
        .filter(|value| value.chars().nth(4) == Some('-') && value.chars().nth(7) == Some('-'));
    fs::write(
        output_dir.join("sessions.json"),
        serde_json::to_vec_pretty(&json!({"snapshot_boundary":boundary,"sessions":sessions}))?,
    )?;
    fs::write(
        output_dir.join("catalog.json"),
        serde_json::to_vec_pretty(&catalog_export(&c)?)?,
    )?;
    fs::write(
        output_dir.join("progress.json"),
        serde_json::to_vec_pretty(&weekly_progress(&c, progress_as_of)?)?,
    )?;
    fs::write(
        output_dir.join("effective-summary.json"),
        serde_json::to_vec_pretty(&effective_evidence_summary(&c)?)?,
    )?;
    let source_manifest = json!({"available_sources":["sqlite:sessions","sqlite:attempts","sqlite:practice_items","sqlite:observations","sqlite:findings","sqlite:reviews","sqlite:production_sessions"],"missing_sources":["original ChatGPT conversation export was not present in the repository or database snapshot","binary/audio/image originals are unavailable unless represented by a source URI in activity_stimuli","tutor summaries cannot substitute for unavailable original source"],"assessment_rule":"unavailable dimensions remain unknown and are not inferred from polished text"});
    fs::write(
        output_dir.join("source-manifest.json"),
        serde_json::to_vec_pretty(&source_manifest)?,
    )?;
    let manifest = ExportManifest {
        snapshot_path: snapshot.display().to_string(),
        snapshot_hash: hash,
        snapshot_boundary: boundary,
        schema_version,
        generated_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        counts,
        available_sources: vec![
            "sqlite:sessions".into(),
            "sqlite:attempts".into(),
            "sqlite:practice_items".into(),
            "sqlite:observations".into(),
            "sqlite:findings".into(),
            "sqlite:reviews".into(),
            "sqlite:production_sessions".into(),
        ],
        missing_sources: vec![
            "original conversation export".into(),
            "unavailable binary/audio/image originals".into(),
        ],
    };
    fs::write(
        output_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest)?,
    )?;
    Ok(manifest)
}

fn proposal(
    run_id: &str,
    proposal_id: &str,
    entity_type: &str,
    entity_id: &str,
    operation: AuditOperation,
    before: Value,
    after: Value,
    source_refs: Vec<SourceRef>,
    reason: &str,
    issue_class: &str,
    certainty: Certainty,
    affected_consumers: &[&str],
) -> Result<AuditProposal> {
    let expected_revision = before["revision"].as_i64().unwrap_or(1);
    Ok(AuditProposal {
        audit_run_id: run_id.to_owned(),
        proposal_id: proposal_id.to_owned(),
        schema_version: AUDIT_SCHEMA_VERSION,
        entity_type: entity_type.to_owned(),
        entity_id: entity_id.to_owned(),
        expected_revision,
        before_hash: json_hash(&before)?,
        operation,
        before,
        after,
        source_refs,
        reason: reason.to_owned(),
        issue_class: issue_class.to_owned(),
        certainty,
        reviewer: "operator-assisted-source-review-v1".to_owned(),
        prompt_version: "historical-review-20260912-v1".to_owned(),
        reviewed_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
        dependencies: Vec::new(),
        affected_consumers: affected_consumers.iter().map(|x| (*x).to_owned()).collect(),
    })
}

/// Build the first reviewed batch from facts that are mechanically supported
/// by the snapshot: exact duplicate text, explicit continuation notes, and the
/// stored word-count convention.  Linguistic cases that need the missing
/// original conversation or media remain outside this batch.
pub fn known_repair_batch(db_path: &Path, c: &Connection) -> Result<AuditBatch> {
    let hash = snapshot_hash(db_path)?;
    let run_id = format!("historical-review-{}", &hash[..12]);
    let boundary: String = c.query_row(
        "SELECT COALESCE(max(created_at),'') FROM sessions",
        [],
        |r| r.get(0),
    )?;
    let prompt_version = "historical-review-20260912-v1".to_owned();
    let mut proposals = Vec::new();
    let shared_instance = "exercise-instance-dele-picture-family-dog-20260907";
    let shared_stimulus = "stimulus:family-dog-countryside-photo";
    for sid in [30_i64, 31, 32, 33] {
        let before = entity_value(c, "session", &sid.to_string())?;
        if before["exercise_instance_id"] == shared_instance
            && before["stimulus_identity"] == shared_stimulus
            && before["source_event_id"].is_string()
        {
            continue;
        }
        proposals.push(proposal(
            &run_id,
            &format!("link-picture-sequence-{sid}"),
            "session",
            &sid.to_string(),
            AuditOperation::LinkContinuation,
            before,
            json!({"exercise_instance_id":shared_instance,"stimulus_identity":shared_stimulus,"source_event_id":format!("historical-session-{sid}")}),
            vec![SourceRef { source_id: format!("session:{sid}:notes"), excerpt: Some(if sid == 30 { "Independent spoken-transcript attempt supplied by learner" } else if sid == 31 { "Extended DELE A2-style picture description" } else if sid == 32 { "Continuation of the same DELE A2 picture-description practice" } else { "Second attempt at the same DELE A2-style picture description" }.to_owned()), span: None, source_hash: Some(hash.clone()) }],
            "Link the recorded outdoor family and dog picture sequence while retaining each session and attempt.",
            "continuation_identity",
            Certainty::Supported,
            &["learning_context", "recent_evidence", "practice_brief", "audit_export"],
        )?);
    }
    for oid in [203_i64, 204] {
        let before = entity_value(c, "observation", &oid.to_string())?;
        if before["effective_status"] == "retracted" {
            continue;
        }
        let excerpt = before["produced"].as_str().unwrap_or("").to_owned();
        proposals.push(proposal(
            &run_id,
            &format!("retract-duplicate-observation-{oid}"),
            "observation",
            &oid.to_string(),
            AuditOperation::MarkDuplicate,
            before,
            json!({"effective_status":"retracted"}),
            vec![SourceRef { source_id: format!("observation:{oid}:produced"), excerpt: Some(excerpt), span: None, source_hash: Some(hash.clone()) }],
            "The exact initial transcript and error pair was recorded in session 30 and session 31; preserve session 31 follow-up rehearsal while removing only the duplicated initial evidence contribution.",
            "duplicate_event",
            Certainty::Supported,
            &["weakness_counts", "recent_evidence", "practice_brief", "review_queue", "audit_export"],
        )?);
    }
    let before = entity_value(c, "session", "41")?;
    if before["effective_summary"].is_null() {
        let computed_word_count: i64 = c.query_row(
            "SELECT computed_word_count FROM practice_items WHERE id=82",
            [],
            |r| r.get(0),
        )?;
        proposals.push(proposal(
            &run_id,
            "amend-session-41-objective-summary",
            "session",
            "41",
            AuditOperation::AmendSummary,
            before,
            json!({
                "summary_key":"audit_quality",
                "summary_value":{
                    "status":"corrected",
                    "source":"learner_submitted_response",
                    "word_count":{"value":computed_word_count,"counter_version":word_counter_version(),"convention":"whitespace-separated tokens containing at least one Unicode letter or number; punctuation-only tokens ignored; internal hyphens/apostrophes retained","within_prompt_range":computed_word_count >= 70 && computed_word_count <= 80,"prompt_range":{"min":70,"max":80}},
                    "task_completion":{"place":"present: restaurant mexicano en Centro","decor":"unknown: exact five-point checklist was not present in the snapshot"},
                    "previous_summary_claims":{"within_requested_length":false},
                    "readiness":"not_inferred",
                    "unresolved":["original conversation export and exact five-point prompt checklist are unavailable","linguistic corrections in the tutor note require source-context review before being counted as recovered findings"]
                }
            }),
            vec![
                SourceRef { source_id: "practice_item:82:prompt".into(), excerpt: Some("70-80 palabras".into()), span: None, source_hash: Some(hash.clone()) },
                SourceRef { source_id: "practice_item:82:response".into(), excerpt: Some("La semana pasada mi esposa y yo comemos".into()), span: None, source_hash: Some(hash.clone()) },
            ],
            "Recompute the submitted response against the declared application counter and separate objective length from unavailable checklist evidence.",
            "objective_task_fact",
            Certainty::Supported,
            &["recent_evidence", "learning_context", "practice_brief", "weekly_progress", "audit_export"],
        )?);
    }
    Ok(AuditBatch {
        audit_run_id: run_id,
        snapshot_boundary: boundary,
        snapshot_hash: hash,
        model_id: Some("operator-assisted-source-review-v1".to_owned()),
        prompt_version,
        scope: json!({"kind":"historical_full_snapshot_known_repairs","sessions":"all accessible sessions through snapshot boundary","source_policy":"database source rows only; unavailable conversation/media remain unknown","reviewed_cases":["sessions 30-33 continuation and exact duplicate","session 41 length and task checklist summary"]}),
        proposals,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn application_word_counter_handles_unicode_punctuation_and_internal_marks() {
        assert_eq!(
            word_count("¡Hola, señor!\n¿Cómo estás? Tengo 2 perros. — …"),
            7
        );
        assert_eq!(word_count("madre-hijo d'aprendizaje"), 2);
        assert_eq!(word_count("... — !!!"), 0);
    }

    #[test]
    fn official_configuration_keeps_scoring_groups_separate() {
        let config = official_exam_config();
        assert_eq!(config["tests"]["reading"]["duration_minutes"], 60);
        assert_eq!(config["tests"]["listening"]["duration_minutes"], 40);
        assert_eq!(
            config["tests"]["writing"]["tasks_detail"]["writing_task_1"]["word_min"],
            60
        );
        assert_eq!(
            config["tests"]["writing"]["tasks_detail"]["writing_task_2"]["word_max"],
            80
        );
        assert_eq!(
            config["scoring_groups"]["reading_writing"]["required_points"],
            30
        );
        assert_eq!(
            config["scoring_groups"]["listening_speaking"]["required_points"],
            30
        );
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
