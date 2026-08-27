use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use axum::{
    Json,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header, jwk::JwkSet};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, RwLock};

use crate::config::{AuthConfig, OidcConfig};

use crate::embedded_oauth::SharedOauthState;

use crate::config::EmbeddedOauthConfig;

#[derive(Clone)]
pub enum Authenticator {
    Disabled,
    Oidc(Arc<OidcAuthenticator>),
    Bearer(String),
    EmbeddedOauth(Arc<EmbeddedOauthConfig>, SharedOauthState),
}

#[derive(Clone)]
pub struct OidcAuthenticator {
    config: OidcConfig,
    client: reqwest::Client,
    keys: Arc<RwLock<JwkSet>>,
    last_jwks_refresh: Arc<Mutex<Instant>>,
}

#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    scope: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
}

impl Authenticator {
    pub async fn new(config: AuthConfig) -> Result<Self> {
        match config {
            AuthConfig::Disabled => Ok(Self::Disabled),
            AuthConfig::Bearer(token) => Ok(Self::Bearer(token)),
            AuthConfig::EmbeddedOauth(config) => {
                let config_arc = Arc::new(config.clone());
                let state = crate::embedded_oauth::EmbeddedOauthState::new(config_arc.clone())?;
                Ok(Self::EmbeddedOauth(config_arc, Arc::new(tokio::sync::Mutex::new(state))))
            }
            AuthConfig::Oidc(config) => {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .build()
                    .context("failed to create OIDC HTTP client")?;
                let keys = fetch_jwks(&client, &config.jwks_url).await?;
                if keys.keys.is_empty() {
                    bail!("OIDC JWKS contains no signing keys");
                }
                Ok(Self::Oidc(Arc::new(OidcAuthenticator {
                    config,
                    client,
                    keys: Arc::new(RwLock::new(keys)),
                    last_jwks_refresh: Arc::new(Mutex::new(Instant::now())),
                })))
            }
        }
    }

    pub fn metadata(&self) -> Option<ProtectedResourceMetadata> {
        match self {
            Self::Oidc(auth) => Some(ProtectedResourceMetadata {
                resource: auth.config.public_base_url.clone(),
                authorization_servers: vec![auth.config.issuer.clone()],
                scopes_supported: vec![auth.config.required_scope.clone()],
            }),
            Self::EmbeddedOauth(config, _) => {
                Some(ProtectedResourceMetadata {
                    resource: config.public_base_url.clone(),
                    authorization_servers: vec![config.public_base_url.clone()],
                    scopes_supported: vec![config.required_scope.clone()],
                })
            },
            _ => None,
        }
    }

    fn challenge(&self) -> Option<String> {
        match self {
            Self::Oidc(auth) => Some(format!(
                "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\", scope=\"{}\"",
                auth.config.public_base_url, auth.config.required_scope
            )),
            Self::EmbeddedOauth(config, _) => {
                Some(format!(
                    "Bearer resource_metadata=\"{}/.well-known/oauth-protected-resource\", scope=\"{}\"",
                    config.public_base_url, config.required_scope
                ))
            },
            Self::Bearer(_) => Some("Bearer".to_string()),
            Self::Disabled => None,
        }
    }

    async fn verify(&self, token: &str) -> Result<()> {
        if let Self::Bearer(secret) = self {
            return if constant_time_eq(token, secret) {
                Ok(())
            } else {
                bail!("invalid bearer token")
            };
        }
        if matches!(self, Self::Disabled) {
            return Ok(());
        }
        if token.len() > 16_384 {
            bail!("JWT is too large");
        }
        let header = decode_header(token).context("invalid JWT header")?;
        if !matches!(
            header.alg,
            Algorithm::RS256 | Algorithm::RS384 | Algorithm::RS512
        ) {
            bail!("unsupported JWT signing algorithm");
        }
        let kid = header.kid.context("JWT header has no kid")?;

        match self {
            Self::EmbeddedOauth(config, auth) => {
                let auth = auth.lock().await;
                let jwk = if kid == "1" {
                    Some(auth.rsa_public_jwk.clone())
                } else {
                    None
                };
                let key = DecodingKey::from_jwk(&jwk.context("JWT signing key is unknown")?)
                    .context("invalid JWK")?;
                let mut validation = Validation::new(header.alg);
                validation.set_issuer(&[&config.public_base_url]);
                validation.set_audience(&[&config.public_base_url]);
                let claims = decode::<Claims>(token, &key, &validation)
                    .context("JWT validation failed")?
                    .claims;
                if claims.sub != config.username {
                    bail!("JWT subject is not authorized for this learner");
                }
                let has_scope = claims
                    .scope
                    .as_deref()
                    .unwrap_or_default()
                    .split_ascii_whitespace()
                    .any(|scope| scope == config.required_scope);
                if !has_scope {
                    bail!("JWT does not contain the required scope");
                }
                Ok(())
            }
            Self::Oidc(auth) => {
                let mut jwk = auth.keys.read().await.find(&kid).cloned();
                if jwk.is_none() {
                    let mut last_refresh = auth.last_jwks_refresh.lock().await;
                    if last_refresh.elapsed() >= Duration::from_secs(60) {
                        let refreshed = fetch_jwks(&auth.client, &auth.config.jwks_url).await?;
                        jwk = refreshed.find(&kid).cloned();
                        *auth.keys.write().await = refreshed;
                        *last_refresh = Instant::now();
                    }
                }
                let key = DecodingKey::from_jwk(jwk.as_ref().context("JWT signing key is unknown")?)
                    .context("invalid JWK")?;
                let mut validation = Validation::new(header.alg);
                validation.set_issuer(&[&auth.config.issuer]);
                validation.set_audience(&[&auth.config.audience]);
                let claims = decode::<Claims>(token, &key, &validation)
                    .context("JWT validation failed")?
                    .claims;
                if claims.sub != auth.config.allowed_subject {
                    bail!("JWT subject is not authorized for this learner");
                }
                let has_scope = claims
                    .scope
                    .as_deref()
                    .unwrap_or_default()
                    .split_ascii_whitespace()
                    .any(|scope| scope == auth.config.required_scope);
                if !has_scope {
                    bail!("JWT does not contain the required scope");
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub async fn require_auth(
    State(auth): State<Authenticator>,
    request: Request,
    next: Next,
) -> Response {
    if matches!(auth, Authenticator::Disabled) {
        return next.run(request).await;
    }
    let token = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "));
    if let Some(token) = token
        && auth.verify(token).await.is_ok()
    {
        return next.run(request).await;
    }
    let mut response = (StatusCode::UNAUTHORIZED, "authentication required").into_response();
    if let Some(challenge) = auth.challenge()
        && let Ok(value) = HeaderValue::from_str(&challenge)
    {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

async fn fetch_jwks(client: &reqwest::Client, url: &str) -> Result<JwkSet> {
    client
        .get(url)
        .send()
        .await
        .context("failed to fetch OIDC JWKS")?
        .error_for_status()
        .context("OIDC JWKS endpoint returned an error")?
        .json::<JwkSet>()
        .await
        .context("OIDC JWKS response is invalid")
}

pub async fn protected_resource_metadata(State(auth): State<Authenticator>) -> impl IntoResponse {
    match auth.metadata() {
        Some(metadata) => (StatusCode::OK, Json(metadata)).into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}
