use aprendiendo_mcp::{
    db::{LearningStore, SqliteStore},
    model::RecordPracticeSessionRequest,
    production,
};
use serde_json::{Value, json};

async fn store() -> SqliteStore {
    let path = std::env::temp_dir().join(format!("spontaneous-{}.sqlite3", uuid::Uuid::new_v4()));
    let s = SqliteStore::new(path).unwrap();
    s.production_action("update",json!({"patch":production::approved(),"expected_version":0,"source":"Explicit test learner preferences"})).await.unwrap();
    for key in ["duration_llevar_gerund", "para_que_subjunctive"] {
        s.upsert_weakness_json(json!({"key":key,"category":"grammar","description":"Give an update on an ongoing activity","target_pattern":"llevar + gerundio","primary_concept_key":"form.grammar.verb_system.periphrasis","active":true})).await.unwrap();
    }
    s
}
async fn brief(s: &SqliteStore, v: Value) -> Value {
    s.practice_brief(serde_json::from_value(v).unwrap())
        .await
        .unwrap()
}
fn plan(prompt: &str) -> Value {
    json!({"policy_version":1,"turns":[{"turn_no":1,"prompt":prompt,"drill_type":"situational_response","target_weakness_keys":["duration_llevar_gerund"]}],"target_opportunities":[{"weakness_key":"duration_llevar_gerund","communicative_function":"Give an update"}],"planned_initial_sentences":3})
}
fn record() -> Value {
    json!({"idempotency_key":"independent-round","exercise_type_key":"production_drill","topic":"Recent experiences",
 "items":[{"item_no":1,"drill_type":"situational_response","prompt":"Tu compañero necesita una actualización del problema. ¿Qué le dices?","target_weakness_keys":["duration_llevar_gerund"]},{"item_no":2,"drill_type":"question_answer","prompt":"¿Qué ha cambiado últimamente en tus aficiones?","target_weakness_keys":["duration_llevar_gerund"]}],
 "attempts":[{"attempt_no":1,"practice_item_no":1,"transcript":"Llevo dos horas investigando el problema.","response_mode":"typed","observations":[{"observation_no":1,"weakness_key":"duration_llevar_gerund","outcome":"correct","role":"targeted","assessment_phase":"cold_retrieval","hint_level":"none","evidence_strength":"spontaneous_production","learner_effort":"some_effort"}]},{"attempt_no":2,"practice_item_no":2,"transcript":"Llevo un mes aprendiendo a pintar.","response_mode":"typed","observations":[{"observation_no":2,"weakness_key":"duration_llevar_gerund","outcome":"correct","role":"targeted","assessment_phase":"cold_retrieval","hint_level":"none","evidence_strength":"spontaneous_production","learner_effort":"some_effort"}]}],
 "reviews":[{"weakness_key":"duration_llevar_gerund","rating":"good","rating_rationale":"Learner reports successful retrieval with some effort in both situations.","retrieval_mode":"spontaneous_production","evidence_strength":"spontaneous_production","evidence_observation_nos":[1,2]}],
 "production_evidence":{"policy_version":1,"target_opportunities":[{"weakness_key":"duration_llevar_gerund","communicative_function":"Explain ongoing activity"}],"attempts":[{"attempt_no":1,"attempt_kind":"initial","prompt_cueing":"none_detected","prior_target_exposure":"none_known","communicative_outcome":"successful","assessment_provenance":{"kind":"tutor_judgment","evidence":"Open update; no relevant wording previously supplied"},"scenario_tag":"coworker_update","topic_tag":"recent_problem","initial_sentence_count":2},{"attempt_no":2,"attempt_kind":"initial","prompt_cueing":"none_detected","prior_target_exposure":"none_known","communicative_outcome":"successful","assessment_provenance":{"kind":"tutor_judgment","evidence":"Open personal question; no relevant assistance"},"scenario_tag":"personal_hobby_change","topic_tag":"hobby","initial_sentence_count":1}],"observations":[{"observation_no":1,"attempt_no":1,"target_realization":"used","target_accuracy":"correct","assessment_provenance":{"kind":"tutor_judgment","evidence":"llevo dos horas investigando"}},{"observation_no":2,"attempt_no":2,"target_realization":"used","target_accuracy":"correct","assessment_provenance":{"kind":"tutor_judgment","evidence":"llevo un mes aprendiendo"}}],"interventions":[],"variation_assessments":[{"weakness_key":"duration_llevar_gerund","observation_nos":[1,2],"rationale":"A time-sensitive coworker update versus a self-chosen change in a personal hobby; distinct communicative situations, not substituted nouns."}]}})
}
async fn save(s: &SqliteStore, v: Value) -> aprendiendo_mcp::model::RecordPracticeSessionResponse {
    s.record_practice(serde_json::from_value(v).unwrap())
        .await
        .unwrap()
}

#[tokio::test]
async fn saved_policy_filters_inheritance_and_overrides_are_explicit() {
    let s = store().await;
    for req in [
        json!({"count":3,"drill_mix":"production_focused"}),
        json!({"count":3,"drill_mix":"production_focused","allowed_drill_types":["situational_response","question_answer","micro_story"],"activity_type":"role_play_complications","activity_config":{"unpredictable_followups":true,"complication_count":1}}),
    ] {
        let b = brief(&s, req).await;
        assert_eq!(b["preference_version"], 1);
        assert!(b["conflicts"].as_array().unwrap().is_empty(), "{b}");
        for t in b["tutor_context"]["targets"].as_array().unwrap() {
            assert!(
                t["recommended_drill_types"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|d| d != "sentence_transformation" && d != "sentence_completion")
            );
        }
        assert_eq!(
            b["target_opportunities"][0]["elicitation_requirement"],
            "optional"
        );
        assert!(b["activity_plan"]["item_allocations"].is_null());
    }
    let b = brief(
        &s,
        json!({"activity_type":"sentence_transformation_sprint"}),
    )
    .await;
    assert_eq!(b["status"], "no_compatible_activity");
    let b = brief(&s, json!({"allowed_drill_types":["translation"]})).await;
    assert_eq!(b["status"], "no_compatible_activity");
    let b=brief(&s,json!({"practice_mode":"controlled","drill_mix":"translation_only","session_overrides":{"patch":{"default_practice_mode":"controlled","excluded_drill_types":[]},"source":"Learner explicitly requested translation today"}})).await;
    assert_eq!(
        b["effective_preferences"]["default_practice_mode"],
        "controlled"
    );
    assert_eq!(
        s.production_action("get", json!({})).await.unwrap()["preferences"]["default_practice_mode"],
        "spontaneous"
    );
    let ctx = s
        .learning_context(serde_json::from_value(json!({"recent_sessions":0})).unwrap())
        .await
        .unwrap();
    assert_eq!(ctx["recent_sessions"], json!([]));
    assert_eq!(ctx["practice_policy"]["preference_version"], 1);
    let stale=s.production_action("update",json!({"patch":{"written_round_budget":{"maximum":4}},"expected_version":0,"source":"Learner wants shorter rounds"})).await.unwrap_err();
    assert!(stale.to_string().contains("conflict"));
    let updated=s.production_action("update",json!({"patch":{"written_round_budget":{"maximum":4}},"expected_version":1,"source":"Learner wants shorter rounds"})).await.unwrap();
    assert_eq!(updated["preferences"]["written_round_budget"]["minimum"], 3);
}
#[tokio::test]
async fn validator_rejects_cues_without_function_word_false_positive() {
    let s = store().await;
    for prompt in [
        "Exprésalo con llevar + tiempo + gerundio",
        "¿Cuánto tiempo llevas con eso?",
    ] {
        let v = s.production_action("validate", plan(prompt)).await.unwrap();
        assert_eq!(v["status"], "revision_required");
        assert!(v["checks"][0]["evidence_span"].is_string());
    }
    for prompt in [
        "Tu compañero necesita una actualización del problema que estás investigando. ¿Qué le dices?",
        "¿Qué te gustaría hacer para descansar?",
    ] {
        let v = s.production_action("validate", plan(prompt)).await.unwrap();
        assert_eq!(v["status"], "structural_checks_passed");
        assert_eq!(v["semantic_review"], "required");
    }
    let mut p = plan("Cuéntame algo de ayer.");
    p["turns"][0]["model_answer"] = json!("Llevo una hora esperando.");
    assert_eq!(
        s.production_action("validate", p).await.unwrap()["status"],
        "revision_required"
    );
}
#[tokio::test]
async fn independent_review_applies_and_replays_after_preference_change() {
    let s = store().await;
    let v = record();
    let first = save(&s, v.clone()).await;
    assert_eq!(first.review_updates.len(), 1);
    assert!(first.review_decisions[0].eligible);
    s.production_action("update",json!({"patch":{"written_round_budget":{"maximum":4}},"expected_version":1,"source":"Learner requests smaller round"})).await.unwrap();
    let replay = save(&s, v.clone()).await;
    assert_eq!(first.session_id, replay.session_id);
    assert_eq!(first.review_decisions, replay.review_decisions);
    assert_eq!(first.review_updates, replay.review_updates);
    let mut changed = v;
    changed["topic"] = json!("changed");
    assert!(
        s.record_practice(serde_json::from_value(changed).unwrap())
            .await
            .unwrap_err()
            .to_string()
            .contains("idempotency_conflict")
    );
    let recent = s
        .recent_practice(serde_json::from_value(json!({"detail":"full"})).unwrap())
        .await
        .unwrap();
    let text = serde_json::to_string(&recent).unwrap();
    assert!(text.contains("initial_turn_denominator"));
    assert!(text.contains("coworker_update"));
    assert!(text.contains("none_detected"));
}
#[tokio::test]
async fn completed_cued_evidence_is_saved_and_not_scheduled() {
    let s = store().await;
    let mut v = record();
    v["items"][0]["prompt"] = json!("¿Cuánto tiempo llevas con eso?");
    let out = save(&s, v).await;
    assert!(out.review_updates.is_empty());
    assert!(
        out.review_decisions[0]
            .reason_codes
            .contains(&"prompt_supplied_target".into())
    );
    let recent = s
        .recent_practice(serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    assert!(recent.to_string().contains("cued_controlled"));
}
#[tokio::test]
async fn alternative_wording_is_not_a_failure_or_queue_error() {
    let s = store().await;
    let mut v = record();
    v["attempts"][0]["transcript"] = json!("Empecé hace dos horas.");
    v["attempts"][0]["observations"][0]["outcome"] = json!("omitted");
    v["production_evidence"]["observations"][0]["target_realization"] = json!("not_observed");
    v["production_evidence"]["observations"][0]["target_accuracy"] = json!("not_assessable");
    let out = save(&s, v).await;
    assert!(out.review_updates.is_empty());
    assert!(
        out.review_decisions[0]
            .reason_codes
            .contains(&"target_not_observed".into())
    );
    let ctx = s
        .learning_context(serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap();
    for w in ctx["active_weaknesses"].as_array().unwrap() {
        assert_eq!(w["incorrect_count"], 0);
    }
}
#[tokio::test]
async fn retry_preserves_failure_and_model_exposes_transfer() {
    let s = store().await;
    let mut v = record();
    v["attempts"][0]["transcript"] = json!("Llevo dos horas investigo.");
    v["attempts"][0]["observations"][0]["outcome"] = json!("incorrect");
    v["production_evidence"]["observations"][0]["target_accuracy"] = json!("incorrect");
    let mut retry = v["attempts"][0].clone();
    retry["attempt_no"] = json!(3);
    retry["transcript"] = json!("Llevo dos horas investigando.");
    retry["observations"][0]["observation_no"] = json!(3);
    retry["observations"][0]["outcome"] = json!("correct");
    retry["observations"][0]["assessment_phase"] = json!("immediate_retry");
    retry["observations"][0]["hint_level"] = json!("indirect");
    v["attempts"].as_array_mut().unwrap().push(retry);
    let mut a = v["production_evidence"]["attempts"][0].clone();
    a["attempt_no"] = json!(3);
    a["attempt_kind"] = json!("self_correction");
    a["original_attempt_no"] = json!(1);
    v["production_evidence"]["attempts"]
        .as_array_mut()
        .unwrap()
        .push(a);
    let mut o = v["production_evidence"]["observations"][1].clone();
    o["observation_no"] = json!(3);
    o["attempt_no"] = json!(3);
    v["production_evidence"]["observations"]
        .as_array_mut()
        .unwrap()
        .push(o);
    v["production_evidence"]["interventions"] = json!([{"intervention_no":1,"kind":"indirect_hint","text":"Dijiste: Llevo dos horas investigo. Revisa la forma del segundo verbo.","target_weakness_keys":["duration_llevar_gerund"],"after_attempt_no":2,"before_attempt_no":3}]);
    // Hint shown after the completed two-turn round; original is attempt 1.

    v["reviews"][0]["evidence_observation_nos"] = json!([1, 2, 3]);
    let out = save(&s, v.clone()).await;
    assert!(out.review_updates.is_empty());
    assert!(
        out.review_decisions[0]
            .reason_codes
            .contains(&"assisted_retry".into())
    );
    v["idempotency_key"] = json!("transfer-after-model");
    v["production_evidence"]["interventions"][0]["kind"] = json!("model_correction");
    v["production_evidence"]["interventions"][0]["after_attempt_no"] = json!(1);
    v["production_evidence"]["attempts"][1]["attempt_kind"] = json!("fresh_transfer");
    let out = save(&s, v).await;
    assert!(out.review_updates.is_empty());
}
#[tokio::test]
async fn invalid_links_are_atomic_and_unknown_metadata_never_schedules() {
    let s = store().await;
    let mut v = record();
    v["production_evidence"]["observations"][0]["attempt_no"] = json!(99);
    assert!(
        s.record_practice(serde_json::from_value(v).unwrap())
            .await
            .is_err()
    );
    assert_eq!(s.data_status().await.unwrap()["counts"]["sessions"], 0);
    let mut v = record();
    v["production_evidence"]["attempts"][0]["prompt_cueing"] = json!("unknown");
    assert!(save(&s, v).await.review_updates.is_empty());
}
#[test]
fn example_has_a_typed_contract() {
    let _: RecordPracticeSessionRequest = serde_json::from_value(record()).unwrap();
}

#[tokio::test]
async fn documented_end_to_end_round_retains_hint_retry_and_failure() {
    let s = store().await;
    let b = brief(&s, json!({"count":3,"drill_mix":"production_focused"})).await;
    let request: Value = serde_json::from_str(include_str!(
        "../examples/spontaneous-production-session.json"
    ))
    .unwrap();
    let checked = s
        .production_action(
            "validate",
            plan(request["items"][0]["prompt"].as_str().unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(checked["status"], "structural_checks_passed");
    let response = save(&s, request).await;
    assert_eq!(response.review_updates.len(), 1);
    assert_eq!(response.review_updates[0].rating, "again");
    assert_eq!(
        response.review_decisions[0].supporting_observation_nos,
        vec![1, 2]
    );
    assert_eq!(
        response.review_decisions[0].excluded_observation_nos,
        vec![3]
    );
    let recent = s
        .recent_practice(serde_json::from_value(json!({"detail":"full"})).unwrap())
        .await
        .unwrap();
    if let Ok(path) = std::env::var("SPONTANEOUS_EXAMPLE_OUTPUT") {
        std::fs::write(path,serde_json::to_string_pretty(&json!({"brief":b,"validation":checked,"recording":response,"recent_practice":recent})).unwrap()).unwrap();
    }
    assert!(recent.to_string().contains("self_correction"));
}

#[test]
fn approved_seed_is_idempotent_and_other_databases_keep_defaults() {
    let mut c = rusqlite::Connection::open_in_memory().unwrap();
    aprendiendo_mcp::migrations::run(&mut c).unwrap();
    assert_eq!(production::get(&c).unwrap()["version"], 0);
    production::seed(&c).unwrap();
    production::update(&c,serde_json::from_value(json!({"patch":{"written_round_budget":{"maximum":4}},"expected_version":1,"source":"Later learner edit"})).unwrap()).unwrap();
    let preserved = production::seed(&c).unwrap();
    assert_eq!(preserved["version"], 2);
    assert_eq!(
        preserved["preferences"]["written_round_budget"]["maximum"],
        4
    );
}

#[tokio::test]
async fn insufficient_effort_variety_and_one_cue_are_skipped() {
    for change in ["effort", "variety", "single", "easy"] {
        let s = store().await;
        let mut v = record();
        match change {
            "effort" => v["attempts"][0]["observations"][0]["learner_effort"] = json!("unknown"),
            "variety" => {
                v["production_evidence"]["attempts"][1]["scenario_tag"] = json!("coworker_update")
            }
            "single" => v["reviews"][0]["evidence_observation_nos"] = json!([1]),
            _ => v["reviews"][0]["rating"] = json!("easy"),
        }
        let out = save(&s, v).await;
        assert!(out.review_updates.is_empty(), "{change}");
        assert!(!out.review_decisions[0].eligible);
        assert_eq!(out.attempt_count, 2);
    }
}

#[tokio::test]
async fn invalid_accuracy_and_cross_target_reviews_are_atomic() {
    for change in ["accuracy", "wrong_target", "missing_observation"] {
        let s = store().await;
        let mut v = record();
        match change {
            "accuracy" => {
                v["production_evidence"]["observations"][0]["target_realization"] =
                    json!("not_observed")
            }
            "wrong_target" => v["reviews"][0]["weakness_key"] = json!("para_que_subjunctive"),
            _ => v["reviews"][0]["evidence_observation_nos"] = json!([99]),
        }
        assert!(
            s.record_practice(serde_json::from_value(v).unwrap())
                .await
                .is_err()
        );
        assert_eq!(s.data_status().await.unwrap()["counts"]["sessions"], 0);
    }
}

#[tokio::test]
async fn exhausted_formats_do_not_fallback_and_spoken_timing_stays_unknown() {
    let s = store().await;
    let out = brief(&s, json!({"allowed_drill_types":[]})).await;
    assert_eq!(out["status"], "no_compatible_activity");
    let mut v = record();
    v["attempts"][0]["response_mode"] = json!("spoken_transcript");
    save(&s, v).await;
    let recent = s
        .recent_practice(serde_json::from_value(json!({"detail":"full"})).unwrap())
        .await
        .unwrap();
    assert!(!recent.to_string().contains("words_per_minute"));
    assert!(
        recent
            .to_string()
            .contains("\"response_latency_milliseconds\":null")
    );
}

#[tokio::test]
async fn opportunity_function_does_not_copy_grammar_description() {
    let s = store().await;
    s.upsert_weakness_json(json!({"key":"duration_llevar_gerund","category":"grammar","description":"Use llevar + gerundio","primary_concept_key":"form.grammar.verb_system.periphrasis","concept_links":[{"concept_key":"use.express_duration","role":"function"}]})).await.unwrap();
    let b = brief(
        &s,
        json!({"weakness_keys":["duration_llevar_gerund"],"count":1}),
    )
    .await;
    let function = b["target_opportunities"][0]["communicative_function"]
        .as_str()
        .unwrap();
    assert!(!function.contains("llevar"));
    assert!(!function.contains("gerundio"));
    assert!(b["tutor_context"]["targets"][0].get("allocation").is_none());
}
