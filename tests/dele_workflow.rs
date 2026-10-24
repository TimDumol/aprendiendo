use aprendiendo_mcp::{
    db::{LearningStore, SqliteStore},
    model::RecordPracticeSessionRequest,
};
use rusqlite::Connection;
use serde_json::json;

#[tokio::test]
async fn progress_counts_first_attempts_per_task_and_skill() {
    let path = std::env::temp_dir().join(format!("dele-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = SqliteStore::new(&path).unwrap();
    let request: RecordPracticeSessionRequest = serde_json::from_value(json!({
        "idempotency_key":"mixed-skills", "exercise_type_key":"production_drill", "session_date":"2026-09-20", "reviewed_at":"2026-09-20T12:00:00Z",
        "items":[{"item_no":1,"drill_type":"micro_story","prompt":"Escribe."},{"item_no":2,"drill_type":"situational_response","prompt":"Habla."}],
        "attempts":[
            {"attempt_no":1,"practice_item_no":1,"transcript":"Hola.","response_mode":"typed"},
            {"attempt_no":2,"practice_item_no":1,"transcript":"Hola, Ana.","response_mode":"typed"},
            {"attempt_no":3,"practice_item_no":2,"transcript":"Buenos días.","response_mode":"spoken_transcript"}
        ],
        "task_requirements":[
            {"practice_item_no":1,"skill":"writing","task_key":"writing_task_1","source":"teacher_created"},
            {"practice_item_no":2,"skill":"speaking","task_key":"oral_task_3","source":"teacher_created"}
        ]
    })).unwrap();
    store.record_practice(request).await.unwrap();
    let c = Connection::open(&path).unwrap();
    let progress = aprendiendo_mcp::audit::weekly_progress(&c, Some("2026-09-20")).unwrap();
    assert_eq!(progress["skills"]["writing"]["first_attempts_in_window"], 1);
    assert_eq!(
        progress["skills"]["speaking"]["first_attempts_in_window"],
        1
    );
    assert_eq!(progress["skills"]["reading"]["first_attempts_in_window"], 0);
    assert_eq!(
        progress["skills"]["listening"]["first_attempts_in_window"],
        0
    );
    assert_eq!(progress["next_priorities"], json!(["writing", "speaking"]));
}

#[test]
fn official_timing_distinguishes_task_from_test() {
    use aprendiendo_mcp::audit::exam_task_constraints;
    let oral = exam_task_constraints("oral_task_3").unwrap();
    assert_eq!(oral["duration_min_seconds"], 180);
    assert_eq!(oral["duration_max_seconds"], 240);
    assert_eq!(oral["shared_preparation_seconds"], 720);
    let writing = exam_task_constraints("writing_task_1").unwrap();
    assert!(writing["duration_seconds"].is_null());
    assert_eq!(writing["test_duration_seconds"], 2700);
}

#[tokio::test]
async fn exam_brief_works_without_weaknesses_and_respects_time_budget() {
    let path = std::env::temp_dir().join(format!("dele-plan-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = SqliteStore::new(&path).unwrap();
    let brief = store
        .practice_brief(serde_json::from_value(json!({"exam_task":"writing_task_1"})).unwrap())
        .await
        .unwrap();
    assert_eq!(brief["status"], "exam_task_practice");
    assert_eq!(brief["recommended"]["taskKey"], "writing_task_1");
    assert_eq!(brief["policy"]["exam_task"]["word_min"], 60);
    store.production_action("update", json!({"expected_version":0,"source":"Test learner focus","patch":{"learning_focus":{"skills":["writing","speaking"],"daily_minutes":6,"exam_practice_percent":80}}})).await.unwrap();
    let plan = store
        .production_action("study_plan", json!({}))
        .await
        .unwrap();
    assert_eq!(plan["recommended"]["skill"], "speaking");
    assert_eq!(plan["available_minutes"], 6);
}

#[tokio::test]
async fn mobile_learning_api_requires_auth_and_replays_exact_submission() {
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
    let path = std::env::temp_dir().join(format!("dele-api-{}.sqlite3", uuid::Uuid::new_v4()));
    let store = Arc::new(SqliteStore::new(&path).unwrap());
    let app = learning_api::router(store).route_layer(middleware::from_fn_with_state(
        Authenticator::Bearer("test-token".into()),
        require_auth,
    ));
    let unauthorized = app
        .clone()
        .oneshot(
            Request::get("/api/practice/v1/learning/plan")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    let payload = json!({"idempotency_key":"mobile:offline-attempt","reviewed_at":"2026-09-19T01:00:00Z","exercise_type_key":"production_drill","task_ref":"writing-invitation-v1","turns":[{"drill_type":"micro_story","prompt":"Contesta a Ana.","attempts":[{"transcript":"Hola, Ana. Gracias por invitarme.","response_mode":"typed"}]}],"task_requirements":[{"practice_item_no":1,"skill":"writing","task_key":"writing_task_1","source":"teacher_created","requested_points":["greet"]}],"task_assessments":[{"attempt":{"turn":1,"attempt":1},"point_key":"greet","status":"met","supporting_excerpt":"Hola, Ana.","evaluator":"learner_self_report"}]});
    let mut session = None;
    for status in ["created", "replayed"] {
        let response = app
            .clone()
            .oneshot(
                Request::post("/api/practice/v1/learning/sessions")
                    .header("authorization", "Bearer test-token")
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 100000).await.unwrap()).unwrap();
        assert_eq!(result["status"], status);
        if let Some(id) = session {
            assert_eq!(result["session_id"], id);
        }
        session = Some(result["session_id"].clone());
        assert_eq!(result["review_updates"], json!([]));
    }
    let mut changed = payload;
    changed["turns"][0]["attempts"][0]["transcript"] = json!("Different text");
    let response = app
        .clone()
        .oneshot(
            Request::post("/api/practice/v1/learning/sessions")
                .header("authorization", "Bearer test-token")
                .header("content-type", "application/json")
                .body(Body::from(changed.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let c = Connection::open(path).unwrap();
    assert_eq!(
        c.query_row("SELECT count(*) FROM sessions", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(
        c.query_row("SELECT session_date FROM sessions", [], |r| r
            .get::<_, String>(0))
            .unwrap(),
        "2026-09-18"
    );
}
