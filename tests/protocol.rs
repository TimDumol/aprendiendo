use std::sync::Arc;

use aprendiendo_mcp::{db::SqliteStore, server::LearningServer};
use axum::Router;
use rmcp::transport::streamable_http_server::{
    StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn post_rpc(client: &reqwest::Client, url: &str, payload: Value) -> Value {
    client
        .post(url)
        .header("accept", "application/json, text/event-stream")
        .json(&payload)
        .send()
        .await
        .expect("request should succeed")
        .error_for_status()
        .expect("MCP should return success")
        .json()
        .await
        .expect("response should be JSON")
}

#[tokio::test]
async fn streamable_http_lists_and_calls_domain_tools() {
    let cancellation = CancellationToken::new();
    let database_path = std::env::temp_dir().join(format!(
        "aprendiendo-protocol-{}.sqlite3",
        uuid::Uuid::new_v4()
    ));
    let template = LearningServer::new(Arc::new(SqliteStore::new(&database_path).unwrap()), None);
    let service: StreamableHttpService<LearningServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(template.clone()),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_legacy_session_mode(false)
                .with_json_response(true)
                .with_cancellation_token(cancellation.child_token()),
        );
    let app = Router::new().nest_service("/mcp", service);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listener should bind");
    let address = listener.local_addr().expect("listener should have address");
    let task = tokio::spawn(async move { axum::serve(listener, app).await });
    let url = format!("http://{address}/mcp");
    let client = reqwest::Client::new();

    let initialized = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "integration-test", "version": "1"}
            }
        }),
    )
    .await;
    assert_eq!(
        initialized["result"]["serverInfo"]["name"],
        "aprendiendo-mcp"
    );

    let listed = post_rpc(
        &client,
        &url,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    )
    .await;
    let tools = listed["result"]["tools"]
        .as_array()
        .expect("tools should be an array");
    assert_eq!(tools.len(), 13);
    assert_eq!(tools[0]["_meta"]["securitySchemes"][0]["type"], "noauth");
    let serialized = serde_json::to_string(tools).expect("tools should serialize");
    assert!(!serialized.contains("projectId"));
    assert!(!serialized.contains("databaseName"));
    assert!(!serialized.contains("run_sql"));
    let record_tool = tools
        .iter()
        .find(|tool| tool["name"] == "record_practice_session")
        .expect("record tool should be listed");
    let input_schema = &record_tool["inputSchema"];
    assert_eq!(
        input_schema["properties"]["items"]["items"]["type"],
        "object"
    );
    assert!(input_schema["$defs"]["ProductionEvidence"]["properties"]["interventions"].is_object());
    assert!(input_schema["properties"]["exercise_type_key"]["enum"].is_array());
    assert!(input_schema["properties"]["attempts"]["items"]["properties"]["observations"]["items"]["properties"]["outcome"]["enum"].is_array());
    let required = input_schema["required"]
        .as_array()
        .expect("record input schema should have required fields");
    assert!(required.iter().any(|field| field == "exercise_type_key"));
    assert!(!required.iter().any(|field| field == "exercise_type"));
    assert!(input_schema["properties"].get("exercise_type").is_none());
    assert_eq!(input_schema["properties"]["items"]["maxItems"], 100);
    assert_eq!(
        input_schema["$defs"]["ObservationInput"]["properties"]["observation_no"]["minimum"],
        1
    );
    assert_eq!(
        input_schema["$defs"]["ActivityRunInput"]["properties"]["planned_duration_seconds"]["maximum"],
        7200
    );
    assert_eq!(
        input_schema["$defs"]["AttemptInput"]["properties"]["response_latency_milliseconds"]["maximum"],
        3_600_000
    );
    assert!(
        record_tool["description"]
            .as_str()
            .unwrap()
            .contains("idempotency")
    );
    assert_eq!(record_tool["annotations"]["idempotentHint"], true);
    assert!(
        record_tool["description"]
            .as_str()
            .unwrap()
            .contains("Advanced canonical fallback")
    );
    assert!(
        record_tool["description"]
            .as_str()
            .unwrap()
            .contains("record_tutoring_session")
    );
    let compact_tool = tools
        .iter()
        .find(|tool| tool["name"] == "record_tutoring_session")
        .expect("compact record tool should be listed");
    assert!(
        compact_tool["description"]
            .as_str()
            .unwrap()
            .contains("Preferred normal tutoring recorder")
    );
    let compact_schema = &compact_tool["inputSchema"];
    assert_eq!(compact_schema["properties"]["turns"]["maxItems"], 100);
    assert_eq!(
        compact_schema["$defs"]["TutoringTurnInput"]["properties"]["attempts"]["maxItems"],
        20
    );
    assert_eq!(
        compact_schema["$defs"]["FindingKind"]["enum"],
        json!([
            "error",
            "awkward",
            "regional_variant",
            "stylistic_improvement",
            "accepted"
        ])
    );
    assert!(
        compact_schema["$defs"]["TutoringAttemptReference"]["properties"]["turn"]["minimum"]
            .is_number()
    );
    let recent_tool = tools
        .iter()
        .find(|tool| tool["name"] == "get_recent_practice")
        .expect("recent practice tool should be listed");
    assert_eq!(
        recent_tool["inputSchema"]["$defs"]["DetailMode"]["oneOf"][0]["enum"],
        json!(["summary", "full"])
    );
    assert_eq!(
        recent_tool["inputSchema"]["$defs"]["DetailMode"]["oneOf"][1]["const"],
        "evidence"
    );
    assert!(recent_tool["inputSchema"]["properties"]["session_id"]["minimum"].is_number());
    assert_eq!(
        recent_tool["inputSchema"]["properties"]["task_ref"]["maxLength"],
        160
    );
    let weakness_tool = tools
        .iter()
        .find(|tool| tool["name"] == "upsert_weakness")
        .expect("weakness maintenance tool should be listed");
    assert!(
        weakness_tool["description"]
            .as_str()
            .unwrap()
            .contains("Maintenance only")
    );
    assert!(
        weakness_tool["description"]
            .as_str()
            .unwrap()
            .contains("can activate a new weakness")
    );
    let concept_tool = tools
        .iter()
        .find(|tool| tool["name"] == "upsert_concept")
        .expect("concept maintenance tool should be listed");
    assert!(
        concept_tool["description"]
            .as_str()
            .unwrap()
            .contains("Maintenance only")
    );
    let output_schema = &record_tool["outputSchema"];
    let output_data = &output_schema["$defs"]["RecordPracticeSessionResponse"];
    let output_required = output_data["required"]
        .as_array()
        .expect("typed record output should declare required fields");
    assert!(output_required.iter().any(|field| field == "status"));
    assert!(output_data["properties"].get("recorded").is_none());
    assert!(output_data["properties"].get("idempotent_replay").is_none());
    assert_eq!(
        output_schema["$defs"]["RecordStatus"]["enum"],
        json!(["created", "replayed"])
    );

    let called = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "get_learning_context", "arguments": {"recent_sessions": 3}}
        }),
    )
    .await;
    assert!(called["result"]["structuredContent"]["data"]["recent_sessions"].is_array());

    let update_args = json!({"patch":aprendiendo_mcp::production::approved(),"expected_version":0,"source":"Explicit protocol-test learner preferences"});
    let updated=post_rpc(&client,&url,json!({"jsonrpc":"2.0","id":30,"method":"tools/call","params":{"name":"update_practice_preferences","arguments":update_args}})).await;
    assert_eq!(updated["result"]["structuredContent"]["data"]["version"], 1);
    let stale=post_rpc(&client,&url,json!({"jsonrpc":"2.0","id":31,"method":"tools/call","params":{"name":"update_practice_preferences","arguments":update_args}})).await;
    assert!(
        stale["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("preference_version_conflict")
    );
    let context=post_rpc(&client,&url,json!({"jsonrpc":"2.0","id":32,"method":"tools/call","params":{"name":"get_learning_context","arguments":{"recent_sessions":0}}})).await;
    assert_eq!(
        context["result"]["structuredContent"]["data"]["recent_sessions"],
        json!([])
    );
    assert_eq!(
        context["result"]["structuredContent"]["data"]["practice_policy"]["preference_version"],
        1
    );

    let record_arguments = json!({
        "idempotency_key": "protocol-record-001",
        "exercise_type_key": "translation_drill"
    });
    let first = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "record_practice_session", "arguments": record_arguments.clone()}
        }),
    )
    .await;
    assert_eq!(
        first["result"]["structuredContent"]["data"]["status"],
        "created"
    );
    let replay = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {"name": "record_practice_session", "arguments": record_arguments}
        }),
    )
    .await;
    assert_eq!(
        replay["result"]["structuredContent"]["data"]["status"],
        "replayed"
    );
    let changed = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {"name": "record_practice_session", "arguments": {
                "idempotency_key": "protocol-record-001",
                "exercise_type_key": "translation_drill",
                "topic": "changed"
            }}
        }),
    )
    .await;
    assert_eq!(changed["result"]["isError"], true);
    assert!(
        changed["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("idempotency_conflict")
    );
    let legacy = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "tools/call",
            "params": {"name": "record_practice_session", "arguments": {
                "idempotency_key": "protocol-legacy-001",
                "exercise_type": "translation_drill"
            }}
        }),
    )
    .await;
    assert_eq!(legacy["result"]["isError"], true);
    let unknown_key = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "tools/call",
            "params": {"name": "record_practice_session", "arguments": {
                "idempotency_key": "protocol-unknown-key-001",
                "exercise_type_key": "not_a_canonical_key"
            }}
        }),
    )
    .await;
    assert_eq!(unknown_key["result"]["isError"], true);
    let invalid_compact = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 80,
            "method": "tools/call",
            "params": {"name": "record_tutoring_session", "arguments": {
                "idempotency_key": "protocol-invalid-compact-001",
                "exercise_type_key": "translation_drill",
                "unexpected_field": "sentinel-transcript"
            }}
        }),
    )
    .await;
    assert_eq!(invalid_compact["result"]["isError"], true);
    let unknown_tool = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 81,
            "method": "tools/call",
            "params": {"name": "not_a_registered_tool", "arguments": {}}
        }),
    )
    .await;
    assert!(unknown_tool["error"].is_object());
    let status = post_rpc(
        &client,
        &url,
        json!({
            "jsonrpc": "2.0",
            "id": 9,
            "method": "tools/call",
            "params": {"name": "get_data_status", "arguments": {}}
        }),
    )
    .await;
    assert_eq!(
        status["result"]["structuredContent"]["data"]["counts"]["sessions"],
        1
    );

    task.abort();
    let _ = std::fs::remove_file(database_path);
}

use aprendiendo_mcp::config::EmbeddedOauthConfig;
use aprendiendo_mcp::embedded_oauth::EmbeddedOauthState;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString},
};
use std::fs;

const ED25519_PRIVATE_KEY_PEM: &[u8] = b"-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIGrD/e7uKYqSY4twDEsRfMMuLSrODf14dpTiTK6K1YI0\n-----END PRIVATE KEY-----\n";

async fn setup_embedded_oauth_app() -> (String, reqwest::Client, tokio::task::JoinHandle<()>, String)
{
    fs::write("/tmp/test_ed25519_key.pem", ED25519_PRIVATE_KEY_PEM).unwrap();

    let password = uuid::Uuid::new_v4().to_string();
    let salt = SaltString::from_b64("c2FsdC1mb3ItdGVzdA").unwrap();
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();

    let config = EmbeddedOauthConfig {
        public_base_url: "http://localhost".to_string(),
        username: "test_user".to_string(),
        password_hash,
        ed25519_private_key_path: "/tmp/test_ed25519_key.pem".to_string(),
        client_id: "test_client".to_string(),
        redirect_uri: "http://localhost/callback".to_string(),
        required_scope: "learning:access".to_string(),
    };

    let config_arc = Arc::new(config);
    let state = Arc::new(tokio::sync::Mutex::new(
        EmbeddedOauthState::new(config_arc.clone()).unwrap(),
    ));

    let app = Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            axum::routing::get(aprendiendo_mcp::embedded_oauth::authorization_server_metadata)
                .with_state(config_arc.clone()),
        )
        .route(
            "/oauth/jwks",
            axum::routing::get(aprendiendo_mcp::embedded_oauth::jwks).with_state(state.clone()),
        )
        .route(
            "/oauth/authorize",
            axum::routing::get(aprendiendo_mcp::embedded_oauth::authorize_get)
                .with_state(config_arc.clone())
                .post(aprendiendo_mcp::embedded_oauth::authorize_post)
                .with_state(state.clone()),
        )
        .route(
            "/oauth/token",
            axum::routing::post(aprendiendo_mcp::embedded_oauth::token_post)
                .with_state(state.clone()),
        );

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = format!("http://{}", address);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();

    (url, client, task, password)
}

#[tokio::test]
async fn embedded_oauth_flow() {
    let (base_url, client, task, password) = setup_embedded_oauth_app().await;

    // 1. Metadata
    let meta: Value = client
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            base_url
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(meta["issuer"], "http://localhost");
    assert_eq!(meta["authorization_response_iss_parameter_supported"], true);

    // 2. JWKS
    let jwks: Value = client
        .get(format!("{}/oauth/jwks", base_url))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(jwks["keys"][0]["kid"], "1");

    // 3. Authorize GET (get CSRF token)
    let authorize_url = format!(
        "{}/oauth/authorize?client_id=test_client&redirect_uri=http://localhost/callback&response_type=code&scope=learning:access&code_challenge=challenge123&code_challenge_method=S256",
        base_url
    );
    let resp = client.get(&authorize_url).send().await.unwrap();
    let html = resp.text().await.unwrap();
    assert!(html.contains("csrf_token"));

    // extract csrf
    let csrf_start = html.find("name=\"csrf_token\" value=\"").unwrap() + 25;
    let csrf_end = html[csrf_start..].find("\"").unwrap() + csrf_start;
    let csrf_token = &html[csrf_start..csrf_end];

    // 4. Authorize POST
    // reqwest follows redirects by default. The test redirect_uri is http://localhost/callback,
    // which fails to connect. We need to disable auto-redirects.
    let resp = client
        .post(&authorize_url)
        .form(&[
            ("username", "test_user"),
            ("password", password.as_str()),
            ("csrf_token", csrf_token),
            ("action", "authorize"),
        ])
        .send()
        .await
        .unwrap();

    let final_url = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(final_url.starts_with("http://localhost/callback?code="));
    assert!(final_url.contains("&iss=http://localhost"));

    let code_start = final_url.find("code=").unwrap() + 5;
    let _code = &final_url[code_start..];

    // 5. Token exchange (mock challenge matching, need proper sha256 of verifier)
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(b"verifier123");
    let hash = hasher.finalize();
    let expected_challenge =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, hash);

    // Get new code with correct challenge
    let authorize_url = format!(
        "{}/oauth/authorize?client_id=test_client&redirect_uri=http://localhost/callback&response_type=code&scope=learning:access&code_challenge={}&code_challenge_method=S256",
        base_url, expected_challenge
    );
    let resp = client.get(&authorize_url).send().await.unwrap();
    let html = resp.text().await.unwrap();
    let csrf_start = html.find("name=\"csrf_token\" value=\"").unwrap() + 25;
    let csrf_end = html[csrf_start..].find("\"").unwrap() + csrf_start;
    let csrf_token = &html[csrf_start..csrf_end];

    let resp = client
        .post(&authorize_url)
        .form(&[
            ("username", "test_user"),
            ("password", password.as_str()),
            ("csrf_token", csrf_token),
            ("action", "authorize"),
        ])
        .send()
        .await
        .unwrap();
    let final_url = resp
        .headers()
        .get("location")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let code_start = final_url.find("code=").unwrap() + 5;
    let code_end = final_url[code_start..]
        .find('&')
        .map(|offset| code_start + offset)
        .unwrap_or(final_url.len());
    let code = &final_url[code_start..code_end];

    let token_resp: Value = client
        .post(format!("{}/oauth/token", base_url))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", "test_client"),
            ("redirect_uri", "http://localhost/callback"),
            ("code_verifier", "verifier123"),
        ])
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(token_resp.get("access_token").is_some());
    assert_eq!(token_resp["token_type"], "Bearer");

    task.abort();
}
