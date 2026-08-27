use std::{
    collections::HashMap,
    sync::Arc,
    time::Instant,
};
use tokio::sync::Mutex;
use jsonwebtoken::jwk::Jwk;
use rsa::{RsaPrivateKey, pkcs8::DecodePrivateKey, traits::PublicKeyParts};
use crate::config::EmbeddedOauthConfig;
use anyhow::{Context, Result};
use std::fs;

#[derive(Clone, Debug)]
pub struct AuthCode {
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub expires_at: Instant,
}

pub struct EmbeddedOauthState {
    pub config: Arc<EmbeddedOauthConfig>,
    pub rsa_private_key: RsaPrivateKey,
    pub rsa_public_jwk: Jwk,
    pub auth_codes: HashMap<String, AuthCode>,
}

impl EmbeddedOauthState {
    pub fn new(config: Arc<EmbeddedOauthConfig>) -> Result<Self> {
        let pem = fs::read_to_string(&config.rsa_private_key_path)
            .context("failed to read OAUTH_RSA_KEY_PATH")?;
        let rsa_private_key = RsaPrivateKey::from_pkcs8_pem(&pem)
            .context("failed to parse RSA private key")?;

        let jwk = Jwk {
            common: jsonwebtoken::jwk::CommonParameters {
                public_key_use: Some(jsonwebtoken::jwk::PublicKeyUse::Signature),
                key_operations: None,
                key_algorithm: Some(jsonwebtoken::jwk::KeyAlgorithm::RS256),
                key_id: Some("1".to_string()),
                x509_url: None,
                x509_chain: None,
                x509_sha1_fingerprint: None,
                x509_sha256_fingerprint: None,
            },
            algorithm: jsonwebtoken::jwk::AlgorithmParameters::RSA(
                jsonwebtoken::jwk::RSAKeyParameters {
                    key_type: jsonwebtoken::jwk::RSAKeyType::RSA,
                    n: base64::Engine::encode(
                        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                        rsa_private_key.n().to_bytes_be(),
                    ),
                    e: base64::Engine::encode(
                        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
                        rsa_private_key.e().to_bytes_be(),
                    ),
                }
            )
        };

        Ok(Self {
            config,
            rsa_private_key,
            rsa_public_jwk: jwk,
            auth_codes: HashMap::new(),
        })
    }
}

pub type SharedOauthState = Arc<Mutex<EmbeddedOauthState>>;

use axum::{Json, extract::State, response::IntoResponse, http::StatusCode};
use serde_json::json;

pub async fn authorization_server_metadata(
    State(config): State<Arc<EmbeddedOauthConfig>>,
) -> impl IntoResponse {
    let metadata = json!({
        "issuer": config.public_base_url,
        "authorization_endpoint": format!("{}/oauth/authorize", config.public_base_url),
        "token_endpoint": format!("{}/oauth/token", config.public_base_url),
        "jwks_uri": format!("{}/oauth/jwks", config.public_base_url),
        "scopes_supported": [config.required_scope],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
    });
    (StatusCode::OK, Json(metadata))
}

pub async fn jwks(State(state): State<SharedOauthState>) -> impl IntoResponse {
    let jwk = state.lock().await.rsa_public_jwk.clone();
    (StatusCode::OK, Json(json!({ "keys": [jwk] })))
}

use axum::{
    extract::{Query, Form},
    response::{Html, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use uuid::Uuid;
use argon2::{Argon2, PasswordHash, PasswordVerifier};

#[derive(Deserialize, Debug)]
pub struct AuthorizeQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub scope: String,
    pub state: Option<String>,
    pub code_challenge: String,
    pub code_challenge_method: String,
}

#[derive(Deserialize, Debug)]
pub struct LoginForm {
    pub username: Option<String>,
    pub password: Option<String>,
    pub csrf_token: String,
    pub action: String,
}

pub async fn authorize_get(
    State(config): State<Arc<EmbeddedOauthConfig>>,
    jar: CookieJar,
    Query(query): Query<AuthorizeQuery>,
) -> Result<(CookieJar, Html<String>), Response> {

    if query.client_id != config.client_id || query.redirect_uri != config.redirect_uri {
        return Err((StatusCode::BAD_REQUEST, "invalid client_id or redirect_uri").into_response());
    }
    if query.response_type != "code" {
        return Err((StatusCode::BAD_REQUEST, "unsupported response_type").into_response());
    }
    if query.code_challenge_method != "S256" {
        return Err((StatusCode::BAD_REQUEST, "unsupported code_challenge_method").into_response());
    }
    if !query.scope.split_ascii_whitespace().any(|sc| sc == config.required_scope) {
        return Err((StatusCode::BAD_REQUEST, "invalid scope").into_response());
    }

    let csrf_token = Uuid::new_v4().to_string();
    let cookie = Cookie::build(("csrf_token", csrf_token.clone()))
        .http_only(true)
        .secure(true)
        .path("/")
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .build();

    let html = format!(r#"<!DOCTYPE html>
<html>
<head><title>Authorize</title></head>
<body>
<h2>Authorize access</h2>
<form method="post">
  <input type="hidden" name="csrf_token" value="{csrf_token}">
  <div>
    <label>Username</label>
    <input type="text" name="username" required>
  </div>
  <div>
    <label>Password</label>
    <input type="password" name="password" required>
  </div>
  <button type="submit" name="action" value="authorize">Authorize</button>
  <button type="submit" name="action" value="deny" formnovalidate>Deny</button>
</form>
</body>
</html>"#, csrf_token = csrf_token);

    Ok((jar.add(cookie), Html(html)))
}

pub async fn authorize_post(
    State(state): State<SharedOauthState>,
    jar: CookieJar,
    Query(query): Query<AuthorizeQuery>,
    Form(form): Form<LoginForm>,
) -> Result<Response, Response> {
    let expected_csrf = jar.get("csrf_token").map(|c| c.value().to_string());
    if expected_csrf.is_none() || expected_csrf.as_deref() != Some(&form.csrf_token) {
        return Err((StatusCode::FORBIDDEN, "invalid CSRF token").into_response());
    }

    let mut s = state.lock().await;

    if query.client_id != s.config.client_id || query.redirect_uri != s.config.redirect_uri {
        return Err((StatusCode::BAD_REQUEST, "invalid client_id or redirect_uri").into_response());
    }

    if form.action == "deny" {
        let mut redirect = format!("{}?error=access_denied", query.redirect_uri);
        if let Some(st) = query.state {
            redirect.push_str(&format!("&state={}", st));
        }
        return Ok(Redirect::to(&redirect).into_response());
    }

    let username = form.username.unwrap_or_default();
    let password = form.password.unwrap_or_default();

    let username_ok = username == s.config.username;
    let parsed_hash_result = PasswordHash::new(&s.config.password_hash);

    let password_ok = match parsed_hash_result {
        Ok(parsed_hash) => Argon2::default().verify_password(password.as_bytes(), &parsed_hash).is_ok(),
        Err(_) => false,
    };

    if !username_ok || !password_ok {
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        return Err((StatusCode::UNAUTHORIZED, "invalid username or password").into_response());
    }

    let code = Uuid::new_v4().to_string();

    s.auth_codes.insert(code.clone(), AuthCode {
        code: code.clone(),
        client_id: query.client_id,
        redirect_uri: query.redirect_uri.clone(),
        code_challenge: query.code_challenge,
        code_challenge_method: query.code_challenge_method,
        expires_at: std::time::Instant::now() + std::time::Duration::from_secs(300),
    });

    let mut redirect = format!("{}?code={}", query.redirect_uri, code);
    if let Some(st) = query.state {
        redirect.push_str(&format!("&state={}", st));
    }

    let jar = jar.remove(Cookie::from("csrf_token"));

    Ok((jar, Redirect::to(&redirect)).into_response())
}

use serde::Serialize;
use sha2::{Digest, Sha256};
use jsonwebtoken::{encode, EncodingKey, Header};

#[derive(Deserialize, Debug)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_verifier: String,
}

#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub scope: String,
}

#[derive(Serialize)]
struct JwtClaims {
    iss: String,
    sub: String,
    aud: String,
    exp: usize,
    scope: String,
}

pub async fn token_post(
    State(state): State<SharedOauthState>,
    Form(req): Form<TokenRequest>,
) -> impl IntoResponse {
    if req.grant_type != "authorization_code" {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "unsupported_grant_type"}))).into_response();
    }

    let mut s = state.lock().await;

    // cleanup expired codes
    s.auth_codes.retain(|_, v| v.expires_at > std::time::Instant::now());

    let auth_code = match s.auth_codes.remove(&req.code) {
        Some(c) => c,
        None => return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response(),
    };

    if auth_code.client_id != req.client_id || auth_code.redirect_uri != req.redirect_uri {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
    }

    let mut hasher = Sha256::new();
    hasher.update(req.code_verifier.as_bytes());
    let hash = hasher.finalize();
    let expected_challenge = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        hash,
    );

    if auth_code.code_challenge != expected_challenge {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "invalid_grant"}))).into_response();
    }

    // Generate JWT
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs();
    let expires_in = 3600;

    let claims = JwtClaims {
        iss: s.config.public_base_url.clone(),
        sub: s.config.username.clone(),
        aud: s.config.public_base_url.clone(),
        exp: (now + expires_in) as usize,
        scope: s.config.required_scope.clone(),
    };

    let mut header = Header::new(jsonwebtoken::Algorithm::RS256);
    header.kid = Some("1".to_string());

    let pem = std::fs::read_to_string(&s.config.rsa_private_key_path).expect("failed to read private key");
    let encoding_key = EncodingKey::from_rsa_pem(pem.as_bytes())
        .expect("failed to create encoding key from pem");

    let token = encode(&header, &claims, &encoding_key).expect("failed to sign token");

    let response = TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in,
        scope: s.config.required_scope.clone(),
    };

    (StatusCode::OK, Json(response)).into_response()
}
