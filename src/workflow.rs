//! Durable tutor handoffs. Feedback attaches to a frozen source, never rewrites
//! learner attempts and never updates FSRS.
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub const DDL: &str = "
CREATE TABLE IF NOT EXISTS tutor_handoffs (
 id TEXT PRIMARY KEY, workspace_ref TEXT NOT NULL UNIQUE, session_id INTEGER NOT NULL REFERENCES sessions(id),
 snapshot_json TEXT NOT NULL, created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE TABLE IF NOT EXISTS tutor_feedback (
 id INTEGER PRIMARY KEY, handoff_id TEXT NOT NULL UNIQUE REFERENCES tutor_handoffs(id),
 idempotency_key TEXT NOT NULL UNIQUE, body_json TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE TABLE IF NOT EXISTS tutor_repairs (
 id INTEGER PRIMARY KEY, handoff_id TEXT NOT NULL REFERENCES tutor_handoffs(id),
 repair_no INTEGER NOT NULL, response TEXT NOT NULL, idempotency_key TEXT NOT NULL UNIQUE,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')));
CREATE INDEX IF NOT EXISTS tutor_repairs_handoff ON tutor_repairs(handoff_id,repair_no);
";

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateHandoff {
    pub workspace_ref: String,
    pub session_id: i64,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HandoffRequest {
    pub handoff_id: String,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutorRepair {
    /// Exact attempt_no from get_tutor_handoff's frozen snapshot.
    pub attempt_no: i64,
    /// Verbatim nonempty excerpt from that learner attempt.
    pub original: String,
    pub hint: String,
    pub suggestion: String,
    pub explanation: String,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutorPoint {
    pub attempt_no: i64,
    pub point_key: String,
    pub status: PointStatus,
    pub supporting_excerpt: Option<String>,
    pub reason: String,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PointStatus {
    Met,
    Partial,
    Missing,
    Unclear,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TutorFeedback {
    pub handoff_id: String,
    pub idempotency_key: String,
    /// Task completion and communication first. No inferred audio judgments.
    pub summary: String,
    #[serde(default)]
    #[schemars(length(max = 2))]
    pub repairs: Vec<TutorRepair>,
    #[serde(default)]
    #[schemars(length(max = 40))]
    pub point_assessments: Vec<TutorPoint>,
}
#[derive(Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SaveRepair {
    pub idempotency_key: String,
    /// One-based index into the returned feedback's repairs.
    pub repair_no: usize,
    pub response: String,
}
fn text(value: &str, name: &str, max: usize) -> Result<()> {
    if value.trim().is_empty() || value.len() > max {
        bail!("invalid: {name} must be nonempty and at most {max} bytes")
    }
    Ok(())
}
fn snapshot(c: &Connection, sid: i64) -> Result<Value> {
    let topic = c
        .query_row("SELECT topic FROM sessions WHERE id=?1", [sid], |r| {
            r.get::<_, Option<String>>(0)
        })
        .optional()?;
    let Some(topic) = topic else {
        bail!("not_found: session")
    };
    let mut q = c.prepare("SELECT a.attempt_no,p.item_no,a.transcript,p.prompt,a.response_mode,a.actual_duration_milliseconds FROM attempts a JOIN practice_items p ON p.id=a.practice_item_id WHERE a.session_id=?1 AND p.effective_status='active' ORDER BY a.attempt_no LIMIT 65")?;
    let attempts = q.query_map([sid], |r| Ok(json!({"attempt_no":r.get::<_,i64>(0)?,"item_no":r.get::<_,i64>(1)?,"transcript":r.get::<_,String>(2)?,"prompt":r.get::<_,String>(3)?,"response_mode":r.get::<_,Option<String>>(4)?,"duration_ms":r.get::<_,Option<i64>>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    if attempts.is_empty() || attempts.len() > 64 {
        bail!("invalid: handoff needs 1–64 active item-linked attempts")
    }
    let out = json!({"session_id":sid,"topic":topic,"attempts":attempts,"task_evidence":crate::curriculum::session_evidence(c,sid)?});
    if serde_json::to_vec(&out)?.len() > 128 * 1024 {
        bail!("invalid: handoff source too large")
    }
    Ok(out)
}
pub fn create(c: &Connection, request: CreateHandoff) -> Result<Value> {
    text(&request.workspace_ref, "workspace_ref", 160)?;
    if request.session_id <= 0 {
        bail!("invalid: positive session_id required")
    }
    let prior: Option<(String, i64)> = c
        .query_row(
            "SELECT id,session_id FROM tutor_handoffs WHERE workspace_ref=?1",
            [&request.workspace_ref],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, sid)) = prior {
        if sid != request.session_id {
            bail!("conflict: workspace already points to another session")
        }
        return get(c, &id);
    }
    let source = snapshot(c, request.session_id)?;
    let id = uuid::Uuid::new_v4().to_string();
    c.execute(
        "INSERT INTO tutor_handoffs(id,workspace_ref,session_id,snapshot_json) VALUES(?1,?2,?3,?4)",
        params![
            id,
            request.workspace_ref,
            request.session_id,
            source.to_string()
        ],
    )?;
    get(c, &id)
}
pub fn get(c: &Connection, id: &str) -> Result<Value> {
    text(id, "handoff_id", 160)?;
    let row: Option<(String,i64,String,String)> = c.query_row("SELECT workspace_ref,session_id,snapshot_json,created_at FROM tutor_handoffs WHERE id=?1",[id],|r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).optional()?;
    let Some((workspace, sid, source, created)) = row else {
        bail!("not_found: handoff")
    };
    let feedback: Option<(i64, String)> = c
        .query_row(
            "SELECT id,body_json FROM tutor_feedback WHERE handoff_id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    let repairs=c.prepare("SELECT repair_no,response,created_at FROM tutor_repairs WHERE handoff_id=?1 ORDER BY id")?.query_map([id], |r| Ok(json!({"repair_no":r.get::<_,i64>(0)?,"response":r.get::<_,String>(1)?,"created_at":r.get::<_,String>(2)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let source: Value = serde_json::from_str(&source)?;
    let source_current = snapshot(c, sid).is_ok_and(|current| current == source);
    Ok(
        json!({"handoff_id":id,"workspace_ref":workspace,"session_id":sid,"snapshot":source,"source_current":source_current,"created_at":created,"feedback":feedback.map(|(fid,body)| -> Result<Value> { Ok(json!({"id":fid,"assessment":serde_json::from_str::<Value>(&body)?})) }).transpose()?,"repair_attempts":repairs,"evidence_class":"tutor_assisted_repair; no automatic FSRS review"}),
    )
}
pub fn feedback(c: &Connection, request: TutorFeedback) -> Result<Value> {
    text(&request.idempotency_key, "idempotency_key", 160)?;
    let body = serde_json::to_string(&request)?;
    if body.len() > 65536 {
        bail!("invalid: feedback too large")
    }
    let prior: Option<(String, String)> = c
        .query_row(
            "SELECT handoff_id,body_json FROM tutor_feedback WHERE idempotency_key=?1",
            [&request.idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if let Some((id, old)) = prior {
        if old != body {
            bail!("conflict: feedback idempotency key has a different payload")
        }
        let mut out = get(c, &id)?;
        out["status"] = json!("replayed");
        return Ok(out);
    }
    text(&request.summary, "summary", 4000)?;
    if request.repairs.len() > 2 || request.point_assessments.len() > 40 {
        bail!("invalid: at most two repairs and forty point assessments")
    }
    let handoff = get(c, &request.handoff_id)?;
    if !handoff["source_current"].as_bool().unwrap_or(false) {
        bail!("conflict: handoff source changed; create a new workspace handoff")
    }
    if !handoff["feedback"].is_null() {
        bail!("conflict: feedback already exists; original assessment is immutable")
    }
    let attempts = handoff["snapshot"]["attempts"]
        .as_array()
        .context("stored handoff contains malformed evidence")?;
    let attempt = |no| {
        attempts
            .iter()
            .find(|a| a["attempt_no"] == no)
            .ok_or_else(|| anyhow::anyhow!("invalid: unknown attempt_no"))
    };
    for repair in &request.repairs {
        let a = attempt(json!(repair.attempt_no))?;
        text(&repair.original, "original", 2000)?;
        text(&repair.hint, "hint", 2000)?;
        text(&repair.suggestion, "suggestion", 2000)?;
        text(&repair.explanation, "explanation", 2000)?;
        if !a["transcript"]
            .as_str()
            .context("stored handoff contains malformed evidence")?
            .contains(&repair.original)
        {
            bail!("invalid: repair excerpt is absent from its attempt")
        }
    }
    let mut assessed = std::collections::HashSet::new();
    for point in &request.point_assessments {
        let a = attempt(json!(point.attempt_no))?;
        text(&point.point_key, "point_key", 500)?;
        text(&point.reason, "reason", 2000)?;
        if !assessed.insert((point.attempt_no, &point.point_key)) {
            bail!("invalid: duplicate point assessment")
        }
        let known = handoff["snapshot"]["task_evidence"]["requirements"]
            .as_array()
            .context("stored handoff contains malformed evidence")?
            .iter()
            .any(|r| {
                r["item_no"] == a["item_no"]
                    && r["requested_points"]
                        .as_array()
                        .is_some_and(|p| p.contains(&json!(point.point_key)))
            });
        if !known {
            bail!("invalid: point is not requested by this attempt's task")
        }
        if let Some(excerpt) = &point.supporting_excerpt {
            text(excerpt, "supporting_excerpt", 2000)?;
            if !a["transcript"]
                .as_str()
                .context("stored handoff contains malformed evidence")?
                .contains(excerpt)
            {
                bail!("invalid: point excerpt is absent from its attempt")
            }
        }
        if matches!(point.status, PointStatus::Met | PointStatus::Partial)
            && point.supporting_excerpt.is_none()
        {
            bail!("invalid: met/partial points require an excerpt")
        }
    }
    c.execute(
        "INSERT INTO tutor_feedback(handoff_id,idempotency_key,body_json) VALUES(?1,?2,?3)",
        params![request.handoff_id, request.idempotency_key, body],
    )?;
    let mut out = get(c, &request.handoff_id)?;
    out["status"] = json!("created");
    Ok(out)
}
pub fn repair(c: &Connection, id: &str, request: SaveRepair) -> Result<Value> {
    text(&request.idempotency_key, "idempotency_key", 160)?;
    text(&request.response, "response", 4000)?;
    let handoff = get(c, id)?;
    let count = handoff["feedback"]["assessment"]["repairs"]
        .as_array()
        .map_or(0, Vec::len);
    if request.repair_no == 0 || request.repair_no > count {
        bail!("invalid: unknown repair_no")
    }
    if !handoff["source_current"].as_bool().unwrap_or(false) {
        bail!("conflict: source is no longer current")
    }
    let prior: Option<(String, usize, String)> = c
        .query_row(
            "SELECT handoff_id,repair_no,response FROM tutor_repairs WHERE idempotency_key=?1",
            [&request.idempotency_key],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((old_id, no, response)) = prior {
        if old_id != id || no != request.repair_no || response != request.response {
            bail!("conflict: repair idempotency key has a different payload")
        }
    } else {
        let count: i64 = c.query_row(
            "SELECT count(*) FROM tutor_repairs WHERE handoff_id=?1",
            [id],
            |r| r.get(0),
        )?;
        if count >= 64 {
            bail!("invalid: repair attempt limit reached")
        }
        c.execute("INSERT INTO tutor_repairs(handoff_id,repair_no,response,idempotency_key) VALUES(?1,?2,?3,?4)",params![id,request.repair_no,request.response,request.idempotency_key])?;
    }
    get(c, id)
}
pub fn actions(c: &Connection) -> Result<Vec<Value>> {
    let mut q=c.prepare("SELECT h.id,h.workspace_ref,h.session_id,f.body_json FROM tutor_handoffs h LEFT JOIN tutor_feedback f ON f.handoff_id=h.id ORDER BY h.created_at DESC LIMIT 20")?;
    let rows = q
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut out = Vec::new();
    for (id, workspace, sid, body) in rows {
        let Some(body) = body else { continue };
        let feedback: Value = serde_json::from_str(&body)?;
        let count = feedback["repairs"].as_array().map_or(0, Vec::len);
        let done: i64 = c.query_row(
            "SELECT count(DISTINCT repair_no) FROM tutor_repairs WHERE handoff_id=?1",
            [&id],
            |r| r.get(0),
        )?;
        if count > done as usize && get(c, &id)?["source_current"] == true {
            out.push(json!({"kind":"repair","handoff_id":id,"workspace_ref":workspace,"session_id":sid,"remaining_repairs":count-done as usize,"label":"Finish tutor repairs"}));
        }
    }
    Ok(out)
}

/// Only the latest recorded task-family session can inform task-gap priority.
/// Self-checklists and stale/retracted handoffs are excluded.
pub fn task_gaps(c: &Connection, key: &str) -> Result<i64> {
    let sid: Option<i64> = c.query_row("SELECT max(t.session_id) FROM task_requirements t JOIN practice_items p ON p.id=t.practice_item_id WHERE t.task_key=?1 AND p.effective_status='active'", [key], |r|r.get(0))?;
    let Some(sid) = sid else { return Ok(0) };
    let ids = c
        .prepare("SELECT id FROM tutor_handoffs WHERE session_id=?1 ORDER BY rowid LIMIT 20")?
        .query_map([sid], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut latest = std::collections::HashMap::<(i64, String), (i64, bool)>::new();
    let mut accept = |item: i64, point: String, attempt: i64, status: &str| {
        let entry = latest.entry((item, point)).or_insert((attempt, false));
        if attempt >= entry.0 {
            *entry = (attempt, matches!(status, "partial" | "missing"));
        }
    };
    let canonical=c.prepare("SELECT p.item_no,a.point_key,at.attempt_no,a.status FROM requested_point_assessments a JOIN attempts at ON at.id=a.attempt_id JOIN practice_items p ON p.id=at.practice_item_id JOIN task_requirements tr ON tr.practice_item_id=p.id WHERE at.session_id=?1 AND tr.task_key=?2 AND p.effective_status='active' AND a.evaluator!='learner_self_report' ORDER BY at.attempt_no,a.id LIMIT 300")?.query_map(params![sid,key],|r|Ok((r.get::<_,i64>(0)?,r.get::<_,String>(1)?,r.get::<_,i64>(2)?,r.get::<_,String>(3)?)))?.collect::<rusqlite::Result<Vec<_>>>()?;
    for (item, point, attempt, status) in canonical {
        accept(item, point, attempt, &status)
    }
    for id in ids {
        let handoff = get(c, &id)?;
        if handoff["source_current"] != true {
            continue;
        }
        let Some(points) = handoff["feedback"]["assessment"]["point_assessments"].as_array() else {
            continue;
        };
        for point in points {
            let a = handoff["snapshot"]["attempts"]
                .as_array()
                .context("stored handoff contains malformed evidence")?
                .iter()
                .find(|a| a["attempt_no"] == point["attempt_no"]);
            let Some(a) = a else { continue };
            if handoff["snapshot"]["task_evidence"]["requirements"]
                .as_array()
                .context("stored handoff contains malformed evidence")?
                .iter()
                .any(|r| r["item_no"] == a["item_no"] && r["task_key"] == key)
            {
                accept(
                    a["item_no"]
                        .as_i64()
                        .context("stored handoff contains malformed evidence")?,
                    point["point_key"]
                        .as_str()
                        .context("stored handoff contains malformed evidence")?
                        .to_owned(),
                    point["attempt_no"]
                        .as_i64()
                        .context("stored handoff contains malformed evidence")?,
                    point["status"]
                        .as_str()
                        .context("stored handoff contains malformed evidence")?,
                );
            }
        }
    }
    Ok(latest.values().filter(|(_, gap)| *gap).count() as i64)
}
