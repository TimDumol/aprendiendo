use crate::config::EmbeddedOauthConfig;
use anyhow::{Context, Result};
use jsonwebtoken::{Algorithm, EncodingKey, jwk::Jwk};
use std::fs;
use std::{collections::HashMap, sync::Arc, time::Instant};
use tokio::sync::Mutex;

const SIGNING_KEY_ID: &str = "1";

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
    pub signing_key: EncodingKey,
    pub public_jwk: Jwk,
    pub auth_codes: HashMap<String, AuthCode>,
}

impl EmbeddedOauthState {
    pub fn new(config: Arc<EmbeddedOauthConfig>) -> Result<Self> {
        let pem = fs::read(&config.ed25519_private_key_path)
            .context("failed to read OAUTH_ED25519_KEY_PATH")?;
        let signing_key =
            EncodingKey::from_ed_pem(&pem).context("failed to parse Ed25519 private key")?;
        let mut jwk = Jwk::from_encoding_key(&signing_key, Algorithm::EdDSA)
            .context("failed to derive Ed25519 public JWK")?;
        jwk.common.public_key_use = Some(jsonwebtoken::jwk::PublicKeyUse::Signature);
        jwk.common.key_id = Some(SIGNING_KEY_ID.to_string());

        Ok(Self {
            config,
            signing_key,
            public_jwk: jwk,
            auth_codes: HashMap::new(),
        })
    }
}

pub type SharedOauthState = Arc<Mutex<EmbeddedOauthState>>;

use axum::{Json, extract::State, http::StatusCode, response::IntoResponse};
use serde_json::json;

pub async fn authorization_server_metadata(
    State(config): State<Arc<EmbeddedOauthConfig>>,
) -> impl IntoResponse {
    let metadata = json!({
        "issuer": config.public_base_url,
        "authorization_response_iss_parameter_supported": true,
        "authorization_endpoint": format!("{}/oauth/authorize", config.public_base_url),
        "token_endpoint": format!("{}/oauth/token", config.public_base_url),
        "jwks_uri": format!("{}/oauth/jwks", config.public_base_url),
        "scopes_supported": [config.required_scope],
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
    });
    (StatusCode::OK, Json(metadata))
}

pub async fn jwks(State(state): State<SharedOauthState>) -> impl IntoResponse {
    let jwk = state.lock().await.public_jwk.clone();
    (StatusCode::OK, Json(json!({ "keys": [jwk] })))
}

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    extract::{Form, Query},
    response::{Html, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar};
use serde::Deserialize;
use tracing::{info, warn};
use uuid::Uuid;

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

#[allow(clippy::result_large_err)]
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
    if !query
        .scope
        .split_ascii_whitespace()
        .any(|sc| sc == config.required_scope)
    {
        return Err((StatusCode::BAD_REQUEST, "invalid scope").into_response());
    }

    let csrf_token = Uuid::new_v4().to_string();
    let cookie = Cookie::build(("csrf_token", csrf_token.clone()))
        .http_only(true)
        .secure(true)
        .path("/")
        .same_site(axum_extra::extract::cookie::SameSite::Lax)
        .build();

    let html = format!(
        r#"<!DOCTYPE html>
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
</html>"#,
        csrf_token = csrf_token
    );

    Ok((jar.add(cookie), Html(html)))
}

#[allow(clippy::result_large_err)]
pub async fn authorize_post(
    State(state): State<SharedOauthState>,
    jar: CookieJar,
    Query(query): Query<AuthorizeQuery>,
    Form(form): Form<LoginForm>,
) -> Result<Response, Response> {
    let expected_csrf = jar.get("csrf_token").map(|c| c.value().to_string());
    if expected_csrf.is_none() || expected_csrf.as_deref() != Some(&form.csrf_token) {
        warn!(reason = "csrf_mismatch", "authorization request rejected");
        return Err((StatusCode::FORBIDDEN, "invalid CSRF token").into_response());
    }

    let mut s = state.lock().await;

    if query.client_id != s.config.client_id || query.redirect_uri != s.config.redirect_uri {
        warn!(
            reason = "client_or_redirect_mismatch",
            "authorization request rejected"
        );
        return Err((StatusCode::BAD_REQUEST, "invalid client_id or redirect_uri").into_response());
    }

    if form.action == "deny" {
        info!("authorization request denied by user");
        let mut redirect = format!(
            "{}?error=access_denied&iss={}",
            query.redirect_uri, s.config.public_base_url
        );
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
        Ok(parsed_hash) => Argon2::default()
            .verify_password(password.as_bytes(), &parsed_hash)
            .is_ok(),
        Err(_) => false,
    };

    if !username_ok || !password_ok {
        warn!(
            username_matches = username_ok,
            password_matches = password_ok,
            "authorization credentials rejected"
        );
        tokio::time::sleep(std::time::Duration::from_millis(1000)).await;
        return Err((StatusCode::UNAUTHORIZED, "invalid username or password").into_response());
    }

    let code = Uuid::new_v4().to_string();

    s.auth_codes.insert(
        code.clone(),
        AuthCode {
            code: code.clone(),
            client_id: query.client_id,
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge,
            code_challenge_method: query.code_challenge_method,
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(300),
        },
    );
    info!("authorization code issued");

    let mut redirect = format!(
        "{}?code={}&iss={}",
        query.redirect_uri, code, s.config.public_base_url
    );
    if let Some(st) = query.state {
        redirect.push_str(&format!("&state={}", st));
    }

    let jar = jar.remove(Cookie::from("csrf_token"));

    Ok((jar, Redirect::to(&redirect)).into_response())
}

use jsonwebtoken::{Header, encode};
use serde::Serialize;
use sha2::{Digest, Sha256};

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
        warn!(reason = "unsupported_grant_type", "token request rejected");
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "unsupported_grant_type"})),
        )
            .into_response();
    }

    let mut s = state.lock().await;

    // cleanup expired codes
    s.auth_codes
        .retain(|_, v| v.expires_at > std::time::Instant::now());

    let auth_code = match s.auth_codes.remove(&req.code) {
        Some(c) => c,
        None => {
            warn!(reason = "unknown_or_expired_code", "token request rejected");
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "invalid_grant"})),
            )
                .into_response();
        }
    };

    if auth_code.client_id != req.client_id || auth_code.redirect_uri != req.redirect_uri {
        warn!(
            reason = "client_or_redirect_mismatch",
            "token request rejected"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    }

    let mut hasher = Sha256::new();
    hasher.update(req.code_verifier.as_bytes());
    let hash = hasher.finalize();
    let expected_challenge =
        base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, hash);

    if auth_code.code_challenge != expected_challenge {
        warn!(
            reason = "pkce_verification_failed",
            "token request rejected"
        );
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "invalid_grant"})),
        )
            .into_response();
    }

    // Generate JWT
    let now = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
        Ok(duration) => duration.as_secs(),
        Err(error) => {
            warn!(%error, "system clock is before the Unix epoch");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "server_error"})),
            )
                .into_response();
        }
    };
    let expires_in = 3600;

    let claims = JwtClaims {
        iss: s.config.public_base_url.clone(),
        sub: s.config.username.clone(),
        aud: s.config.public_base_url.clone(),
        exp: (now + expires_in) as usize,
        scope: s.config.required_scope.clone(),
    };

    let mut header = Header::new(Algorithm::EdDSA);
    header.kid = Some(SIGNING_KEY_ID.to_string());

    let token = match encode(&header, &claims, &s.signing_key) {
        Ok(token) => token,
        Err(error) => {
            warn!(%error, "failed to sign OAuth access token");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "server_error"})),
            )
                .into_response();
        }
    };

    let response = TokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in,
        scope: s.config.required_scope.clone(),
    };

    info!(expires_in, "access token issued");

    (StatusCode::OK, Json(response)).into_response()
}
