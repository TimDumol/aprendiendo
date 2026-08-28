use std::sync::Arc;

use aprendiendo_mcp::{db::MockStore, server::LearningServer};
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
async fn streamable_http_lists_and_calls_six_domain_tools() {
    let cancellation = CancellationToken::new();
    let template = LearningServer::new(Arc::new(MockStore), None);
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
    assert_eq!(tools.len(), 6);
    assert_eq!(tools[0]["_meta"]["securitySchemes"][0]["type"], "noauth");
    let serialized = serde_json::to_string(tools).expect("tools should serialize");
    assert!(!serialized.contains("projectId"));
    assert!(!serialized.contains("databaseName"));
    assert!(!serialized.contains("run_sql"));

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
    assert_eq!(
        called["result"]["structuredContent"]["data"]["recent_sessions"],
        3
    );

    task.abort();
}

use aprendiendo_mcp::config::EmbeddedOauthConfig;
use aprendiendo_mcp::embedded_oauth::EmbeddedOauthState;
use argon2::{
    Argon2,
    password_hash::{PasswordHasher, SaltString, rand_core::OsRng},
};
use rsa::{RsaPrivateKey, pkcs8::EncodePrivateKey};
use std::fs;

async fn setup_embedded_oauth_app() -> (String, reqwest::Client, tokio::task::JoinHandle<()>) {
    let rsa_key = RsaPrivateKey::new(&mut OsRng, 2048).expect("failed to generate key");
    let pem = rsa_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
    fs::write("/tmp/test_key.pem", pem.as_bytes()).unwrap();

    let password = "test_password";
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();

    let config = EmbeddedOauthConfig {
        public_base_url: "http://localhost".to_string(),
        username: "test_user".to_string(),
        password_hash,
        rsa_private_key_path: "/tmp/test_key.pem".to_string(),
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

    (url, client, task)
}

#[tokio::test]
async fn embedded_oauth_flow() {
    let (base_url, client, task) = setup_embedded_oauth_app().await;

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
            ("password", "test_password"),
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
            ("password", "test_password"),
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
