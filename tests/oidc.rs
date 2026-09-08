use std::time::{SystemTime, UNIX_EPOCH};

use aprendiendo_mcp::{
    auth::{Authenticator, ProtectedResourceMetadata, require_auth},
    config::{AuthConfig, OidcConfig},
};
use axum::{
    Json, Router,
    http::StatusCode,
    middleware,
    routing::{get, post},
};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode, jwk::Jwk};
use serde_json::{Value, json};
use tokio::task::JoinHandle;

const ISSUER: &str = "https://auth.example.test";
const RESOURCE: &str = "https://mars.example.test";
const SUBJECT: &str = "learner-subject";
const SCOPE: &str = "learning:access";
const KID: &str = "test-signing-key";
const ED25519_PRIVATE_KEY_DER: &[u8] = &[
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
    0x6a, 0xc3, 0xfd, 0xee, 0xee, 0x29, 0x8a, 0x92, 0x63, 0x8b, 0x70, 0x0c, 0x4b, 0x11, 0x7c, 0xc3,
    0x2e, 0x2d, 0x2a, 0xce, 0x0d, 0xfd, 0x78, 0x76, 0x94, 0xe2, 0x4c, 0xae, 0x8a, 0xd5, 0x82, 0x34,
];

struct TestOidcApp {
    base_url: String,
    auth: Authenticator,
    signing_key: EncodingKey,
    jwks_task: JoinHandle<()>,
    app_task: JoinHandle<()>,
}

impl Drop for TestOidcApp {
    fn drop(&mut self) {
        self.jwks_task.abort();
        self.app_task.abort();
    }
}

async fn setup() -> TestOidcApp {
    let signing_key = EncodingKey::from_ed_der(ED25519_PRIVATE_KEY_DER);
    let mut jwk = Jwk::from_encoding_key(&signing_key, Algorithm::EdDSA)
        .expect("test Ed25519 key should produce a JWK");
    jwk.common.key_id = Some(KID.to_string());
    jwk.common.public_key_use = Some(jsonwebtoken::jwk::PublicKeyUse::Signature);
    let jwks = Json(json!({"keys": [jwk]}));

    let jwks_app = Router::new().route("/jwks", get(move || async move { jwks }));
    let jwks_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("JWKS listener should bind");
    let jwks_url = format!("http://{}/jwks", jwks_listener.local_addr().unwrap());
    let jwks_task = tokio::spawn(async move {
        axum::serve(jwks_listener, jwks_app).await.unwrap();
    });

    let auth = Authenticator::new(AuthConfig::Oidc(OidcConfig {
        public_base_url: RESOURCE.to_string(),
        issuer: ISSUER.to_string(),
        jwks_url,
        audience: RESOURCE.to_string(),
        allowed_subject: SUBJECT.to_string(),
        required_scope: SCOPE.to_string(),
    }))
    .await
    .expect("OIDC authenticator should initialize");

    let app = Router::new()
        .route("/mcp", post(|| async { StatusCode::OK }))
        .layer(middleware::from_fn_with_state(auth.clone(), require_auth));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("app listener should bind");
    let base_url = format!("http://{}", listener.local_addr().unwrap());
    let app_task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    TestOidcApp {
        base_url,
        auth,
        signing_key,
        jwks_task,
        app_task,
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn eddsa_token(key: &EncodingKey, kid: &str, claims: Value) -> String {
    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(kid.to_string());
    encode(&header, &claims, key).expect("test token should encode")
}

fn valid_claims() -> Value {
    json!({
        "iss": ISSUER,
        "aud": RESOURCE,
        "sub": SUBJECT,
        "scope": SCOPE,
        "exp": now() + 300
    })
}

async fn request(app: &TestOidcApp, token: Option<&str>) -> reqwest::Response {
    let client = reqwest::Client::new();
    let mut request = client.post(format!("{}/mcp", app.base_url));
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    request.send().await.unwrap()
}

#[tokio::test]
async fn oidc_metadata_and_missing_token_challenge_are_pocket_id_ready() {
    let app = setup().await;
    let metadata: ProtectedResourceMetadata = app.auth.metadata().unwrap();
    let metadata = serde_json::to_value(metadata).unwrap();
    assert_eq!(metadata["resource"], RESOURCE);
    assert_eq!(metadata["authorization_servers"], json!([ISSUER]));
    assert_eq!(metadata["scopes_supported"], json!([SCOPE]));

    let response = request(&app, None).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let challenge = response
        .headers()
        .get("www-authenticate")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(challenge.contains(
        "resource_metadata=\"https://mars.example.test/.well-known/oauth-protected-resource\""
    ));
    assert!(challenge.contains("scope=\"learning:access\""));
}

#[tokio::test]
async fn oidc_accepts_a_valid_access_token_and_scope_variants() {
    let app = setup().await;
    let token = eddsa_token(&app.signing_key, KID, valid_claims());
    assert_eq!(request(&app, Some(&token)).await.status(), StatusCode::OK);

    let mut claims = valid_claims();
    claims["scope"] = Value::Null;
    claims["scp"] = json!([SCOPE]);
    let token = eddsa_token(&app.signing_key, KID, claims);
    assert_eq!(request(&app, Some(&token)).await.status(), StatusCode::OK);
}

#[tokio::test]
async fn oidc_rejects_invalid_issuer_audience_algorithm_key_scope_subject_and_expiry() {
    let app = setup().await;

    let cases = [
        ("wrong issuer", json!({"iss": "https://wrong.example.test"})),
        (
            "wrong audience",
            json!({"aud": "https://wrong.example.test"}),
        ),
        ("missing scope", json!({"scope": "openid"})),
        ("wrong subject", json!({"sub": "another-subject"})),
        ("expired", json!({"exp": now() - 300})),
        ("id token", json!({"type": "id-token"})),
    ];

    for (name, override_claims) in cases {
        let mut claims = valid_claims();
        for (key, value) in override_claims.as_object().unwrap() {
            claims[key] = value.clone();
        }
        let token = eddsa_token(&app.signing_key, KID, claims);
        assert_eq!(
            request(&app, Some(&token)).await.status(),
            StatusCode::UNAUTHORIZED,
            "case {name} should be rejected"
        );
    }

    let mut wrong_algorithm_header = Header::new(Algorithm::HS256);
    wrong_algorithm_header.kid = Some(KID.to_string());
    let wrong_algorithm = encode(
        &wrong_algorithm_header,
        &valid_claims(),
        &EncodingKey::from_secret(b"not-a-rsa-key"),
    )
    .unwrap();
    assert_eq!(
        request(&app, Some(&wrong_algorithm)).await.status(),
        StatusCode::UNAUTHORIZED
    );

    let unknown_key = eddsa_token(&app.signing_key, "unknown-key", valid_claims());
    assert_eq!(
        request(&app, Some(&unknown_key)).await.status(),
        StatusCode::UNAUTHORIZED
    );
}
