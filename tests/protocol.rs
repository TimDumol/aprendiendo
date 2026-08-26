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
