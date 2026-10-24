use aprendiendo_mcp::{
    db::{LearningStore, SqliteStore},
    workflow,
};
use rusqlite::Connection;
use serde_json::{Value, json};

async fn fixture() -> (SqliteStore, std::path::PathBuf, i64, Value) {
    let path = std::env::temp_dir().join(format!("workflow-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = SqliteStore::new(&path).unwrap();
    let payload = json!({"idempotency_key":"source","exercise_type_key":"production_drill","task_ref":"source-task",
        "items":[{"item_no":1,"drill_type":"micro_story","prompt":"Contesta a Ana."},{"item_no":2,"drill_type":"micro_story","prompt":"Cuenta tu viaje."}],
        "attempts":[{"attempt_no":1,"practice_item_no":1,"transcript":"Hola, Ana. Yo fue al parque.","response_mode":"typed"},{"attempt_no":2,"practice_item_no":2,"transcript":"El viaje fue interesante.","response_mode":"typed"}],
        "task_requirements":[{"practice_item_no":1,"task_key":"writing_task_1","skill":"writing","requested_points":["greet","arrange"],"source":"teacher_created"},{"practice_item_no":2,"task_key":"writing_task_2","skill":"writing","requested_points":["describe"],"source":"teacher_created"}]});
    let sid = store
        .record_practice(serde_json::from_value(payload).unwrap())
        .await
        .unwrap()
        .session_id;
    let handoff = store
        .production_action(
            "handoff_create",
            json!({"workspace_ref":"phone-work","session_id":sid}),
        )
        .await
        .unwrap();
    (store, path, sid, handoff)
}
fn feedback(handoff: &Value) -> Value {
    json!({"handoff_id":handoff["handoff_id"],"idempotency_key":"feedback-1","summary":"You greeted Ana; arranging the visit is still missing.",
        "repairs":[{"attempt_no":1,"original":"Yo fue","hint":"Check the verb ending for yo.","suggestion":"Yo fui","explanation":"Match the subject and verb."}],
        "point_assessments":[{"attempt_no":1,"point_key":"greet","status":"met","supporting_excerpt":"Hola, Ana.","reason":"Direct greeting."},{"attempt_no":1,"point_key":"arrange","status":"missing","reason":"No arrangement was given."}]})
}
#[tokio::test]
async fn feedback_and_repairs_are_exactly_once_and_do_not_change_source_or_fsrs() {
    let (store, path, sid, handoff) = fixture().await;
    let same = store
        .production_action(
            "handoff_create",
            json!({"workspace_ref":"phone-work","session_id":sid}),
        )
        .await
        .unwrap();
    assert_eq!(same["handoff_id"], handoff["handoff_id"]);
    assert_eq!(handoff["snapshot"]["attempts"][0]["attempt_no"], 1);
    let input = feedback(&handoff);
    for expected in ["created", "replayed"] {
        let out = store
            .production_action("handoff_feedback", input.clone())
            .await
            .unwrap();
        assert_eq!(out["status"], expected);
        assert_eq!(out["snapshot"], handoff["snapshot"]);
    }
    let mut changed = input.clone();
    changed["summary"] = json!("Changed assessment");
    assert!(
        store
            .production_action("handoff_feedback", changed)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("conflict:")
    );
    let repair = json!({"handoff_id":handoff["handoff_id"],"request":{"idempotency_key":"repair-1","repair_no":1,"response":"Yo fui"}});
    for _ in 0..2 {
        let result = store
            .production_action("handoff_repair", repair.clone())
            .await
            .unwrap();
        assert_eq!(result["repair_attempts"].as_array().unwrap().len(), 1);
    }
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row(
            "SELECT transcript FROM attempts WHERE attempt_no=1",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "Hola, Ana. Yo fue al parque."
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM weakness_reviews", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(workflow::actions(&c).unwrap().is_empty());
}
#[tokio::test]
async fn wrong_task_excerpts_duplicates_and_stale_sources_are_rejected() {
    let (store, path, _, handoff) = fixture().await;
    let good = feedback(&handoff);
    let mut bad = good.clone();
    bad["repairs"][0]["original"] = json!("Never said this");
    assert!(
        store
            .production_action("handoff_feedback", bad)
            .await
            .is_err()
    );
    let mut bad = good.clone();
    bad["repairs"][0]["attempt_no"] = json!(2);
    assert!(
        store
            .production_action("handoff_feedback", bad)
            .await
            .is_err()
    );
    let mut bad = good.clone();
    bad["point_assessments"][0]["point_key"] = json!("describe");
    assert!(
        store
            .production_action("handoff_feedback", bad)
            .await
            .is_err()
    );
    let mut bad = good.clone();
    bad["point_assessments"][0]["supporting_excerpt"] = Value::Null;
    assert!(
        store
            .production_action("handoff_feedback", bad)
            .await
            .is_err()
    );
    let mut bad = good.clone();
    let point = bad["point_assessments"][0].clone();
    bad["point_assessments"].as_array_mut().unwrap().push(point);
    assert!(
        store
            .production_action("handoff_feedback", bad)
            .await
            .is_err()
    );
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM tutor_feedback", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute(
        "UPDATE practice_items SET effective_status='retracted' WHERE item_no=1",
        [],
    )
    .unwrap();
    assert!(
        store
            .production_action("handoff_feedback", good)
            .await
            .unwrap_err()
            .to_string()
            .starts_with("conflict:")
    );
    assert_eq!(
        workflow::get(&c, handoff["handoff_id"].as_str().unwrap()).unwrap()["source_current"],
        false
    );
}
#[tokio::test]
async fn planner_matches_brief_and_uses_tutor_gaps_and_pending_repairs() {
    let (store, _, _, handoff) = fixture().await;
    store
        .production_action("handoff_feedback", feedback(&handoff))
        .await
        .unwrap();
    store.production_action("update",json!({"expected_version":0,"source":"Test focus","patch":{"learning_focus":{"skills":["writing","speaking"],"daily_minutes":20,"exam_practice_percent":80}}})).await.unwrap();
    let plan = store
        .production_action("study_plan", json!({}))
        .await
        .unwrap();
    let brief = store
        .practice_brief(
            serde_json::from_value(
                json!({"exam_task":"full_writing","planned_duration_seconds":1200}),
            )
            .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(plan["recommended"], brief["recommended"]);
    assert_eq!(plan["recommended"]["taskKey"], "writing_task_1");
    assert_eq!(plan["recommended"]["assessedTaskGaps"], 1);
    assert_eq!(plan["next_actions"][0]["kind"], "repair");
    assert_eq!(plan["next_actions"][0]["workspace_ref"], "phone-work");
}
#[tokio::test]
async fn api_auth_paths_not_found_and_durable_repair_round_trip() {
    use aprendiendo_mcp::{
        auth::{Authenticator, require_auth},
        learning_api,
    };
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        middleware,
    };
    use std::sync::Arc;
    use tower::ServiceExt;
    let (store, _, sid, handoff) = fixture().await;
    store
        .production_action("handoff_feedback", feedback(&handoff))
        .await
        .unwrap();
    let app = learning_api::router(Arc::new(store)).route_layer(middleware::from_fn_with_state(
        Authenticator::Bearer("test-token".into()),
        require_auth,
    ));
    let id = handoff["handoff_id"].as_str().unwrap();
    for (method, path, body) in [
        (
            "POST",
            "/api/practice/v1/learning/handoffs".to_string(),
            json!({"workspace_ref":"phone-work","session_id":sid}),
        ),
        (
            "GET",
            format!("/api/practice/v1/learning/handoffs/{id}"),
            Value::Null,
        ),
        (
            "POST",
            format!("/api/practice/v1/learning/handoffs/{id}/repairs"),
            json!({"idempotency_key":"api-repair","repair_no":1,"response":"Yo fui"}),
        ),
    ] {
        let unauthorized = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(&path)
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let out: Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 200000).await.unwrap()).unwrap();
        assert_eq!(out["handoff_id"], id);
    }
    let missing = app
        .oneshot(
            Request::get("/api/practice/v1/learning/handoffs/missing")
                .header("authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
}
#[tokio::test]
async fn upgrading_schema13_retains_sources_and_idempotency_ledger() {
    let (store, path, _, _) = fixture().await;
    drop(store);
    let mut c = Connection::open(path).unwrap();
    c.execute_batch("DROP TABLE tutor_repairs; DROP TABLE tutor_feedback; DROP TABLE tutor_handoffs; DELETE FROM schema_migrations WHERE version=14; UPDATE schema_meta SET value='13' WHERE key='schema_version';").unwrap();
    let before: i64 = c
        .query_row("SELECT count(*) FROM recorded_requests", [], |r| r.get(0))
        .unwrap();
    aprendiendo_mcp::migrations::run(&mut c).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM recorded_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        before
    );
    assert_eq!(
        c.query_row("SELECT count(*) FROM attempts", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        c.query_row(
            "SELECT value FROM schema_meta WHERE key='schema_version'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "14"
    );
}

#[tokio::test]
async fn next_task_brief_matches_mobile_plan_without_grammar_weaknesses() {
    let path = std::env::temp_dir().join(format!("task-first-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = SqliteStore::new(path).unwrap();
    let plan = store
        .production_action("study_plan", json!({}))
        .await
        .unwrap();
    let brief = store
        .practice_brief(serde_json::from_value(json!({"task_first":true})).unwrap())
        .await
        .unwrap();
    assert_eq!(brief["recommended"], plan["recommended"]);
    assert_eq!(brief["next_actions"], plan["next_actions"]);
    assert_eq!(brief["task_selection_only"], true);
    let scoped = store
        .practice_brief(
            serde_json::from_value(json!({"task_first":true,"exam_task":"writing_task_1"}))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(scoped["recommended"]["taskKey"], "writing_task_1");
    assert_eq!(scoped["task_selection_only"], false);
    assert!(
        store
            .practice_brief(
                serde_json::from_value(json!({"task_first":true,"weakness_keys":["agreement"]}))
                    .unwrap()
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn latest_assessed_revision_replaces_old_gap_even_if_feedback_points_are_unordered() {
    let path = std::env::temp_dir().join(format!(
        "latest-assessment-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let store = SqliteStore::new(&path).unwrap();
    let sid=store.record_practice(serde_json::from_value(json!({"idempotency_key":"repaired-source","exercise_type_key":"production_drill",
        "items":[{"item_no":1,"drill_type":"micro_story","prompt":"Saluda a Ana."}],
        "attempts":[{"attempt_no":1,"practice_item_no":1,"transcript":"Ana.","response_mode":"typed"},{"attempt_no":2,"practice_item_no":1,"transcript":"Hola, Ana.","response_mode":"typed"}],
        "task_requirements":[{"practice_item_no":1,"task_key":"writing_task_1","skill":"writing","requested_points":["greet"],"source":"teacher_created"}],
        "requested_point_assessments":[{"attempt_no":1,"point_key":"greet","status":"partial","supporting_excerpt":"Ana.","evaluator":"tutor_estimate","reason":"No greeting."},{"attempt_no":2,"point_key":"greet","status":"met","supporting_excerpt":"Hola, Ana.","evaluator":"tutor_estimate","reason":"Greeting is present."}]
    })).unwrap()).await.unwrap().session_id;
    let c = Connection::open(&path).unwrap();
    assert_eq!(workflow::task_gaps(&c, "writing_task_1").unwrap(), 0);
    let handoff = store
        .production_action(
            "handoff_create",
            json!({"workspace_ref":"repaired","session_id":sid}),
        )
        .await
        .unwrap();
    store.production_action("handoff_feedback",json!({"handoff_id":handoff["handoff_id"],"idempotency_key":"repaired-feedback","summary":"The revision greets Ana.","repairs":[],"point_assessments":[
        {"attempt_no":2,"point_key":"greet","status":"met","supporting_excerpt":"Hola, Ana.","reason":"Greeting is present."},
        {"attempt_no":1,"point_key":"greet","status":"partial","supporting_excerpt":"Ana.","reason":"No greeting."}
    ]})).await.unwrap();
    assert_eq!(workflow::task_gaps(&c, "writing_task_1").unwrap(), 0);
}
