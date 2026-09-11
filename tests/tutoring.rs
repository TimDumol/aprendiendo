use aprendiendo_mcp::{
    db::{LearningStore, SqliteStore},
    model::{RecordPracticeSessionRequest, RecordStatus},
    production,
    server::LearningServer,
    tutoring::RecordTutoringSessionRequest,
};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::{sync::Arc, time::Instant};

async fn seeded_store(prefix: &str) -> (Arc<SqliteStore>, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "aprendiendo-{prefix}-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let store = Arc::new(SqliteStore::new(&path).unwrap());
    store
        .production_action(
            "update",
            json!({
                "patch": production::approved(),
                "expected_version": 0,
                "source": "Explicit disposable tutoring integration test preferences"
            }),
        )
        .await
        .unwrap();
    store
        .upsert_weakness_json(json!({
            "key": "duration_llevar_gerund",
            "category": "grammar",
            "description": "Use llevar with a gerund to describe an ongoing activity.",
            "target_pattern": "llevar + gerundio",
            "target_type": "grammatical_construction",
            "active": true
        }))
        .await
        .unwrap();
    (store, path)
}

fn compact_fixture() -> RecordTutoringSessionRequest {
    serde_json::from_str(include_str!("../examples/tutoring-compact-session.json")).unwrap()
}

fn canonical_fixture() -> RecordPracticeSessionRequest {
    serde_json::from_str(include_str!("../examples/tutoring-canonical-session.json")).unwrap()
}

fn count(path: &std::path::Path, table: &str) -> i64 {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
}

fn target_status(path: &std::path::Path, key: &str) -> String {
    let connection = Connection::open(path).unwrap();
    connection
        .query_row(
            "SELECT target_status FROM weaknesses WHERE key=?1",
            [key],
            |row| row.get(0),
        )
        .unwrap()
}

#[tokio::test]
async fn compact_record_preserves_atomic_semantics_and_exact_replay() {
    let (store, path) = seeded_store("compact").await;
    let server = LearningServer::new(store.clone(), None);
    let request = compact_fixture();

    let first = server
        .record_tutoring_session(rmcp::handler::server::wrapper::Parameters(request.clone()))
        .await
        .unwrap()
        .0
        .data;
    assert_eq!(first.status, RecordStatus::Created);
    assert_eq!(first.item_count, 2);
    assert_eq!(first.attempt_count, 3);
    assert_eq!(first.observation_count, 4);
    assert_eq!(first.finding_counts["regional_variant"], 1);
    assert_eq!(first.finding_counts["stylistic_improvement"], 1);
    assert!(first.review_updates.is_empty());
    assert!(first.review_decisions.is_empty());

    assert_eq!(count(&path, "sessions"), 1);
    assert_eq!(count(&path, "practice_items"), 2);
    assert_eq!(count(&path, "attempts"), 3);
    assert_eq!(count(&path, "observations"), 4);
    assert_eq!(count(&path, "session_findings"), 2);
    assert_eq!(count(&path, "weakness_reviews"), 0);
    assert_eq!(
        target_status(&path, "incidental.para_que.subjunctive"),
        "candidate"
    );
    assert_eq!(target_status(&path, "duration_llevar_gerund"), "active");

    let recent = store
        .recent_practice(serde_json::from_value(json!({"detail": "full"})).unwrap())
        .await
        .unwrap();
    assert_eq!(recent["items"][0]["attempts"].as_array().unwrap().len(), 3);
    assert_eq!(recent["items"][0]["findings"].as_array().unwrap().len(), 2);
    assert_eq!(
        recent["items"][0]["attempts"][1]["transcript"],
        "Llevo dos horas investigando el problema y cambiaron la reunión."
    );
    assert_eq!(recent["items"][0]["finding_counts"]["regional_variant"], 1);

    store
        .production_action(
            "update",
            json!({
                "patch": {"written_round_budget": {"maximum": 4}},
                "expected_version": 1,
                "source": "Learner requested a shorter round in the replay test"
            }),
        )
        .await
        .unwrap();
    let replay = server
        .record_tutoring_session(rmcp::handler::server::wrapper::Parameters(request.clone()))
        .await
        .unwrap()
        .0
        .data;
    assert_eq!(replay.status, RecordStatus::Replayed);
    assert_eq!(replay.session_id, first.session_id);
    assert_eq!(replay.review_decisions, first.review_decisions);
    assert_eq!(count(&path, "sessions"), 1);

    let mut changed = request.clone();
    changed.topic = Some("Changed after the first write".into());
    let error = server
        .record_tutoring_session(rmcp::handler::server::wrapper::Parameters(changed))
        .await
        .err()
        .expect("changed idempotency payload should fail");
    assert!(error.starts_with("idempotency_conflict:"), "{error}");
    assert_eq!(count(&path, "sessions"), 1);

    let mut invalid = aprendiendo_mcp::tutoring::expand(request).unwrap();
    invalid.idempotency_key = "invalid-finding-reference".into();
    invalid.findings[0].practice_item_no = Some(99);
    let error = store.record_practice(invalid).await.unwrap_err();
    assert!(
        error
            .to_string()
            .contains("references an item not in this request")
    );
    assert_eq!(count(&path, "sessions"), 1);
    assert_eq!(count(&path, "session_findings"), 2);

    drop(server);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn canonical_and_compact_fixtures_persist_the_same_recording_shape() {
    let (compact_store, compact_path) = seeded_store("equivalent-compact").await;
    let compact = compact_store
        .record_practice(aprendiendo_mcp::tutoring::expand(compact_fixture()).unwrap())
        .await
        .unwrap();
    let compact_status = compact_store.data_status().await.unwrap();

    let (canonical_store, canonical_path) = seeded_store("equivalent-canonical").await;
    let canonical = canonical_store
        .record_practice(canonical_fixture())
        .await
        .unwrap();
    let canonical_status = canonical_store.data_status().await.unwrap();

    assert_eq!(compact.item_count, canonical.item_count);
    assert_eq!(compact.attempt_count, canonical.attempt_count);
    assert_eq!(compact.observation_count, canonical.observation_count);
    assert_eq!(compact.finding_counts, canonical.finding_counts);
    assert_eq!(compact.review_updates.len(), canonical.review_updates.len());
    assert_eq!(compact_status["counts"]["session_findings"], 2);
    assert_eq!(
        compact_status["counts"]["observations"],
        canonical_status["counts"]["observations"]
    );
    assert_eq!(
        target_status(&compact_path, "incidental.para_que.subjunctive"),
        "candidate"
    );
    assert_eq!(
        target_status(&canonical_path, "incidental.para_que.subjunctive"),
        "candidate"
    );

    drop(compact_store);
    drop(canonical_store);
    std::fs::remove_file(compact_path).unwrap();
    std::fs::remove_file(canonical_path).unwrap();
}

#[tokio::test]
async fn compact_review_references_expand_to_the_supported_canonical_review() {
    let (store, path) = seeded_store("compact-review").await;
    let mut payload: Value =
        serde_json::from_str(include_str!("../examples/tutoring-compact-session.json")).unwrap();
    payload["idempotency_key"] = json!("tutoring-compact-review-001");
    payload["turns"][0]["attempts"][0]["interventions_after"] = json!([]);
    payload["turns"][0]["attempts"][1]["evidence"]["kind"] = json!("initial");
    payload["turns"][0]["attempts"][1]["evidence"]
        .as_object_mut()
        .unwrap()
        .remove("original_attempt");
    payload["reviews"] = json!([{
        "target": "duration_llevar_gerund",
        "rating": "again",
        "retrieval_mode": "spontaneous_production",
        "evidence_strength": "spontaneous_production",
        "observation_refs": [
            {"turn": 1, "attempt": 1, "observation": 1},
            {"turn": 2, "attempt": 1, "observation": 1}
        ],
        "variation": {
            "observation_refs": [
                {"turn": 1, "attempt": 1, "observation": 1},
                {"turn": 2, "attempt": 1, "observation": 1}
            ],
            "rationale": "A coworker update and a personal hobby question provide distinct communicative situations."
        }
    }]);
    let request: RecordTutoringSessionRequest = serde_json::from_value(payload).unwrap();
    let response = store
        .record_practice(aprendiendo_mcp::tutoring::expand(request).unwrap())
        .await
        .unwrap();
    assert_eq!(response.review_updates.len(), 1);
    assert!(response.review_decisions[0].eligible);
    assert_eq!(
        response.review_decisions[0].supporting_observation_nos,
        vec![1, 4]
    );
    assert_eq!(count(&path, "weakness_reviews"), 1);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn style_only_compact_record_does_not_create_or_schedule_a_weakness() {
    let (store, path) = seeded_store("style-only").await;
    let server = LearningServer::new(store.clone(), None);
    let request: RecordTutoringSessionRequest = serde_json::from_value(json!({
        "idempotency_key": "style-only-001",
        "exercise_type_key": "guided_conversation",
        "turns": [{
            "drill_type": "question_answer",
            "prompt": "¿Qué hiciste este fin de semana?",
            "attempts": [{
                "transcript": "Fui al parque con mis amigos.",
                "findings": [{
                    "assessment_kind": "stylistic_improvement",
                    "original": "con mis amigos",
                    "suggestion": "con unos amigos",
                    "note": "Both are understandable; this is a conversational variant."
                }]
            }]
        }]
    }))
    .unwrap();
    let response = server
        .record_tutoring_session(rmcp::handler::server::wrapper::Parameters(request))
        .await
        .unwrap()
        .0
        .data;
    assert_eq!(response.new_weaknesses_created, Vec::<String>::new());
    assert!(response.review_updates.is_empty());
    assert_eq!(response.finding_counts["stylistic_improvement"], 1);
    assert_eq!(count(&path, "weaknesses"), 1);
    assert_eq!(count(&path, "session_findings"), 1);
    drop(server);
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn policy_version_without_production_facts_keeps_record_only_path() {
    let (store, path) = seeded_store("policy-without-evidence").await;
    let request: RecordTutoringSessionRequest = serde_json::from_value(json!({
        "idempotency_key": "record-only-policy-001",
        "exercise_type_key": "guided_conversation",
        "policy_version": 1,
        "turns": [{
            "drill_type": "question_answer",
            "prompt": "¿Qué hiciste ayer?",
            "attempts": [{"transcript": "Fui al mercado."}]
        }]
    }))
    .unwrap();
    let response = store
        .record_practice(aprendiendo_mcp::tutoring::expand(request).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status, RecordStatus::Created);
    assert!(response.review_updates.is_empty());
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn recent_practice_can_select_exact_or_related_session_with_compact_evidence() {
    let (store, path) = seeded_store("related-retrieval").await;
    let mut request = compact_fixture();
    request.idempotency_key = "related-retrieval-001".into();
    request.task_ref = Some("madrid-dele-task".into());
    let response = store
        .record_practice(aprendiendo_mcp::tutoring::expand(request).unwrap())
        .await
        .unwrap();

    let related = store
        .recent_practice(
            serde_json::from_value(json!({
                "task_ref": "madrid-dele-task",
                "limit": 1,
                "detail": "evidence"
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let item = &related["items"][0];
    assert_eq!(item["id"], response.session_id);
    assert_eq!(item["task_ref"], "madrid-dele-task");
    assert_eq!(item["evidence"]["status"], "recorded");
    assert!(
        item["evidence"]["target_weakness_keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|key| key == "duration_llevar_gerund")
    );
    assert!(item["evidence"]["attempts"][0]["attempt_kind"].is_string());
    assert!(item["evidence"]["attempts"][0]["prompt_cueing"].is_string());
    assert!(item["evidence"]["attempts"][0]["prior_target_exposure"].is_string());
    assert!(item["evidence"]["attempts"][0]["practice_item_no"].is_number());
    assert!(item["evidence"]["attempts"][0]["independence"].is_string());
    assert!(item["evidence"]["attempts"][0]["original_attempt_no"].is_null());
    assert!(item["evidence"]["interventions"][0]["text"].is_string());
    assert_eq!(
        item["evidence"]["observations"][0]["weakness_key"],
        "duration_llevar_gerund"
    );
    assert!(item["evidence"]["observations"][0]["outcome"].is_string());
    assert!(item["evidence"]["observations"][0]["hint_level"].is_string());
    assert!(item["evidence"]["observations"][0]["correction"].is_string());
    assert!(item["evidence"]["observations"][0]["attempt_no"].is_number());
    assert!(item["evidence"]["observations"][0]["target_accuracy"].is_string());
    assert!(item["evidence"]["attempts"][0].get("transcript").is_none());

    let exact = store
        .recent_practice(
            serde_json::from_value(json!({
                "session_id": response.session_id,
                "limit": 1
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(exact["items"].as_array().unwrap().len(), 1);
    assert_eq!(exact["items"][0]["id"], response.session_id);

    let default_recent = store
        .recent_practice(serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(default_recent["items"].as_array().unwrap().len() <= 3);

    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[tokio::test]
async fn local_recording_measurement_reports_created_and_replayed_samples() {
    let (store, path) = seeded_store("measurement").await;
    let mut request = compact_fixture();
    let mut created_ms = Vec::new();
    for index in 0..5 {
        request.idempotency_key = format!("tutoring-measurement-created-{index}");
        let started = Instant::now();
        let response = store
            .record_practice(aprendiendo_mcp::tutoring::expand(request.clone()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status, RecordStatus::Created);
        created_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let replay_request = request.clone();
    let mut replay_ms = Vec::new();
    for _ in 0..5 {
        let started = Instant::now();
        let response = store
            .record_practice(aprendiendo_mcp::tutoring::expand(replay_request.clone()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status, RecordStatus::Replayed);
        replay_ms.push(started.elapsed().as_secs_f64() * 1_000.0);
    }
    let average = |values: &[f64]| values.iter().sum::<f64>() / values.len() as f64;
    let percentile = |values: &mut [f64], fraction: f64| {
        values.sort_by(f64::total_cmp);
        let rank = fraction * (values.len() - 1) as f64;
        let lower = rank.floor() as usize;
        let upper = rank.ceil() as usize;
        values[lower] + (values[upper] - values[lower]) * (rank - lower as f64)
    };
    let mut created_sorted = created_ms.clone();
    let mut replay_sorted = replay_ms.clone();
    eprintln!(
        "local compact recording measurement: created_samples={} created_mean_ms={:.3} created_p50_ms={:.3} created_p95_ms={:.3} replay_samples={} replay_mean_ms={:.3} replay_p50_ms={:.3} replay_p95_ms={:.3}",
        created_ms.len(),
        average(&created_ms),
        percentile(&mut created_sorted, 0.50),
        percentile(&mut created_sorted, 0.95),
        replay_ms.len(),
        average(&replay_ms),
        percentile(&mut replay_sorted, 0.50),
        percentile(&mut replay_sorted, 0.95)
    );
    drop(store);
    std::fs::remove_file(path).unwrap();
}

#[test]
fn compact_fixture_has_no_unknown_fields_and_expands_without_runtime_state() {
    let request = compact_fixture();
    let first = aprendiendo_mcp::tutoring::expand(request.clone()).unwrap();
    let second = aprendiendo_mcp::tutoring::expand(request).unwrap();
    assert_eq!(
        serde_json::to_vec(&first).unwrap(),
        serde_json::to_vec(&second).unwrap()
    );
    let mut unknown: Value =
        serde_json::from_str(include_str!("../examples/tutoring-compact-session.json")).unwrap();
    unknown["unexpected_field"] = json!(true);
    assert!(serde_json::from_value::<RecordTutoringSessionRequest>(unknown).is_err());
}
