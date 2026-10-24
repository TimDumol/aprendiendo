//! Task-first production planning shared by the mobile API and MCP briefs.
use crate::model::{ExamTask, PracticeBriefRequest};
use anyhow::{Context, Result};
use rusqlite::{Connection, params};
use serde_json::{Value, json};

pub fn catalog() -> Result<Vec<Value>> {
    serde_json::from_str(include_str!("../apps/practice/lib/domain/dele-tasks.json"))
        .context("bundled task catalog is invalid JSON")
}

pub fn session_evidence(c: &Connection, session_id: i64) -> Result<Value> {
    let mut q = c.prepare("SELECT t.task_key,t.skill,t.requested_points_json,t.source,p.item_no FROM task_requirements t LEFT JOIN practice_items p ON p.id=t.practice_item_id WHERE t.session_id=?1 AND (p.id IS NULL OR p.effective_status='active')")?;
    let requirements = q.query_map([session_id], |r| Ok(json!({"task_key":r.get::<_,String>(0)?,"skill":r.get::<_,String>(1)?,"requested_points":serde_json::from_str::<Value>(&r.get::<_,String>(2)?).unwrap_or(json!([])),"source":r.get::<_,String>(3)?,"item_no":r.get::<_,Option<i64>>(4)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut q = c.prepare("SELECT a.attempt_no,r.point_key,r.status,r.supporting_excerpt,r.reason,r.evaluator FROM requested_point_assessments r JOIN attempts a ON a.id=r.attempt_id LEFT JOIN practice_items p ON p.id=a.practice_item_id WHERE a.session_id=?1 AND (p.id IS NULL OR p.effective_status='active') ORDER BY a.attempt_no,r.point_key")?;
    let assessments = q.query_map([session_id], |r| Ok(json!({"attempt_no":r.get::<_,i64>(0)?,"point_key":r.get::<_,String>(1)?,"status":r.get::<_,String>(2)?,"supporting_excerpt":r.get::<_,Option<String>>(3)?,"reason":r.get::<_,Option<String>>(4)?,"evaluator":r.get::<_,String>(5)?})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(json!({"requirements":requirements,"point_assessments":assessments}))
}

pub fn plan(c: &Connection, task: Option<ExamTask>, minutes: Option<u16>) -> Result<Value> {
    let preferences = crate::production::get(c)?;
    let focus = &preferences["preferences"]["learning_focus"];
    let available_minutes = minutes
        .map(u64::from)
        .or_else(|| focus["daily_minutes"].as_u64());
    let mut tasks = Vec::new();
    for mut material in catalog()? {
        let key = material["taskKey"]
            .as_str()
            .context("bundled task catalog is missing a required string")?;
        let skill = material["skill"]
            .as_str()
            .context("bundled task catalog is missing a required string")?;
        if let Some(task) = task {
            let matches = match task {
                ExamTask::FullWriting => skill == "writing",
                ExamTask::FullOral => skill == "speaking",
                _ => key == task.as_str(),
            };
            if !matches {
                continue;
            }
        } else if !focus["skills"]
            .as_array()
            .is_some_and(|s| s.contains(&json!(skill)))
        {
            continue;
        }
        let (count, last): (i64, Option<String>) = c.query_row(
            "SELECT count(DISTINCT t.session_id), max(s.session_date) FROM task_requirements t JOIN sessions s ON s.id=t.session_id WHERE t.task_key=?1 AND (t.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=t.practice_item_id AND p.effective_status='active'))",
            [key], |r| Ok((r.get(0)?, r.get(1)?)))?;
        let seen: i64 = c.query_row(
            "SELECT count(*) FROM sessions s WHERE s.task_ref=?1 OR EXISTS(SELECT 1 FROM practice_items p WHERE p.session_id=s.id AND p.prompt=?2 AND p.effective_status='active')",
            [material["id"].as_str().context("bundled task catalog is missing a required string")?, material["prompt"].as_str().context("bundled task catalog is missing a required string")?],
            |r| r.get(0),
        )?;
        let suggested_minutes = match key {
            "writing_task_1" => 15,
            "writing_task_2" => 20,
            "oral_task_3" => 8,
            _ => 6,
        };
        material["assessedTaskGaps"] = json!(crate::workflow::task_gaps(c, key)?);
        material["recordedTaskSessions"] = json!(count);
        material["lastTaskDate"] = json!(last);
        material["materialSeen"] = json!(seen > 0);
        material["materialNovelty"] = json!(if seen == 0 {
            "not_in_recorded_history"
        } else {
            "known_reused_material"
        });
        material["source"] = json!("teacher_created");
        material["suggestedPracticeMinutes"] = json!(suggested_minutes);
        tasks.push(material);
    }
    tasks.sort_by_key(|t| {
        (
            available_minutes
                .is_some_and(|m| t["suggestedPracticeMinutes"].as_u64().unwrap_or(0) > m),
            std::cmp::Reverse(t["assessedTaskGaps"].as_i64().unwrap_or(0)),
            t["recordedTaskSessions"].as_i64().unwrap_or(0),
            t["materialSeen"].as_bool().unwrap_or(false),
            t["lastTaskDate"].as_str().unwrap_or("").to_owned(),
        )
    });
    let mut coverage = Vec::new();
    for key in [
        "writing_task_1",
        "writing_task_2",
        "oral_task_1",
        "oral_task_2",
        "oral_task_3",
    ] {
        let count: i64 = c.query_row(
            "SELECT count(DISTINCT t.session_id) FROM task_requirements t WHERE t.task_key=?1 AND (t.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=t.practice_item_id AND p.effective_status='active'))",
            [key],
            |r| r.get(0),
        )?;
        coverage.push(json!({"task_key":key,"recorded_sessions":count}));
    }
    // A later task in the same family is a transfer opportunity, never automatically
    // independent evidence. Actual exposure and help still need recording.
    let mut followups = c.prepare("SELECT t.task_key,s.task_ref,max(s.session_date) AS last_date FROM task_requirements t JOIN sessions s ON s.id=t.session_id WHERE t.skill IN ('writing','speaking') AND (t.practice_item_id IS NULL OR EXISTS(SELECT 1 FROM practice_items p WHERE p.id=t.practice_item_id AND p.effective_status='active')) GROUP BY t.task_key HAVING last_date <= date('now','-2 days') ORDER BY last_date LIMIT 10")?;
    let followups = followups.query_map(params![], |r| Ok(json!({"task_key":r.get::<_,String>(0)?,"previous_material":r.get::<_,Option<String>>(1)?,"previous_date":r.get::<_,String>(2)?,"instruction":"Use a different situation and material; assess assistance and novelty before claiming transfer."})))?.collect::<rusqlite::Result<Vec<_>>>()?;
    let mut next_actions = crate::workflow::actions(c)?;
    for followup in &followups {
        if let Some(material) = tasks
            .iter()
            .find(|t| t["taskKey"] == followup["task_key"] && t["materialSeen"] == false)
        {
            next_actions.push(json!({"kind":"fresh_task","label":"Try a different situation","task":material,"previous_date":followup["previous_date"],"evidence_class":"fresh-task opportunity; independence unknown"}));
        }
    }
    Ok(
        json!({"generated_at":chrono::Utc::now().to_rfc3339(),"next_actions":next_actions,"goal":crate::audit::learner_goal(c)?,"focus":focus,"available_minutes":available_minutes,"recommended":tasks.first(),"tasks":tasks,"coverage":coverage,"fresh_followups":followups,"evidence_summary":crate::audit::effective_evidence_summary(c)?,"reason":"Prioritize tasks that fit the available time, then assessed task gaps, the least-practiced production task and material absent from recorded history. Practice-minute suggestions are local budgets, not official task limits. Unrecorded exposure is unknown.","feedback_order":["requested points with excerpts","communication and task success","at most two consequential repairs","a different task on a later day"],"readiness":"Practice evidence only; no whole-exam pass probability."}),
    )
}

pub fn exam_brief(c: &Connection, request: &PracticeBriefRequest, policy: Value) -> Result<Value> {
    let minutes = request
        .planned_duration_seconds
        .map(|s| (s / 60).max(1).min(240) as u16);
    let mut out = plan(c, request.exam_task, minutes)?;
    if let Some(fields) = policy.as_object() {
        for (key, value) in fields {
            out[key] = value.clone();
        }
    }
    out["policy"] = policy;
    out["target_opportunities"] = json!([]);
    out["evidence_summary"] = crate::audit::effective_evidence_summary(c)?;
    out["status"] = json!("exam_task_practice");
    out["task_scope"] = json!(request.exam_task);
    out["task_selection_only"] =
        json!(request.task_first == Some(true) && request.exam_task.is_none());
    out["instructions"] = json!([
        "When task_selection_only is true, use this shared recommendation to select the next task, then obtain an exam_task-scoped brief and validate before delivering it. Do not substitute a local grammar-priority ranking.",
        "Select one material per requested task; full writing has two tasks and full oral has three. Catalog prompts are teacher-created, not official exam materials.",
        "Use the material id as task_ref. Record each task requirement and each learner attempt separately.",
        "Exam instructions, stimulus texts, photos and preparation notes are permitted; do not provide model answers or target constructions before the first attempt.",
        "For oral task 3, act as the interlocutor with one adaptive turn at a time. A transcript alone cannot establish pronunciation or measured fluency.",
        "Finish the selected task or mock before coaching. Assess requested points before giving up to two repairs. Revisions do not replace first attempts.",
        "If every material is familiar, generate a new situation and a new stable material id; do not label reused material as fresh."
    ]);
    Ok(out)
}
