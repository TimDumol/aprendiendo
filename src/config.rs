use std::{env, net::SocketAddr, str::FromStr, time::Duration};

use anyhow::{Context, Result, bail};

#[derive(Clone, Debug)]
pub struct Config {
    pub bind_addr: SocketAddr,
    pub database_path: String,
    pub database_timeout: Duration,
    pub max_request_bytes: usize,
    pub auth: AuthConfig,
}

#[derive(Clone, Debug)]
pub enum AuthConfig {
    Disabled,
    Oidc(OidcConfig),
    Bearer(String),
    EmbeddedOauth(EmbeddedOauthConfig),
}

#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub public_base_url: String,
    pub issuer: String,
    pub jwks_url: String,
    pub audience: String,
    pub allowed_subject: String,
    pub required_scope: String,
}

#[derive(Clone, Debug)]
pub struct EmbeddedOauthConfig {
    pub public_base_url: String,
    pub username: String,
    pub password_hash: String,
    pub rsa_private_key_path: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub required_scope: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let bind_addr = env_or("BIND_ADDR", "127.0.0.1:8080")
            .parse::<SocketAddr>()
            .context("BIND_ADDR must be a socket address such as 127.0.0.1:8080")?;
        let database_path = env_or("DATABASE_PATH", "data/aprendiendo.sqlite3");
        let database_timeout_seconds = parse_or("DATABASE_TIMEOUT_SECONDS", 10u64)?;
        let max_request_bytes = parse_or("MAX_REQUEST_BYTES", 131_072usize)?;

        let auth = match env_or("AUTH_MODE", "oidc").as_str() {
            "oidc" => {
                let public_base_url = trim_trailing_slash(required("PUBLIC_BASE_URL")?);
                AuthConfig::Oidc(OidcConfig {
                    audience: env::var("OIDC_AUDIENCE").unwrap_or_else(|_| public_base_url.clone()),
                    public_base_url,
                    issuer: trim_trailing_slash(required("OIDC_ISSUER")?),
                    jwks_url: required("OIDC_JWKS_URL")?,
                    allowed_subject: required("OIDC_ALLOWED_SUBJECT")?,
                    required_scope: env_or("OIDC_REQUIRED_SCOPE", "learning:access"),
                })
            }
            "disabled" => {
                let explicitly_allowed = env_or("ALLOW_INSECURE_NO_AUTH", "false") == "true";
                if !bind_addr.ip().is_loopback() && !explicitly_allowed {
                    bail!(
                        "AUTH_MODE=disabled may only bind to loopback unless ALLOW_INSECURE_NO_AUTH=true"
                    );
                }
                AuthConfig::Disabled
            }
            "bearer" => {
                let token = required("BEARER_TOKEN")?;
                if token.len() < 32 {
                    bail!(
                        "BEARER_TOKEN must be at least 32 characters; generate one with `openssl rand -base64 32`"
                    );
                }
                AuthConfig::Bearer(token)
            }
            "embedded_oauth" => AuthConfig::EmbeddedOauth(EmbeddedOauthConfig {
                public_base_url: trim_trailing_slash(required("PUBLIC_BASE_URL")?),
                username: required("OAUTH_USERNAME")?,
                password_hash: required("OAUTH_PASSWORD_HASH")?,
                rsa_private_key_path: required("OAUTH_RSA_KEY_PATH")?,
                client_id: required("OAUTH_CLIENT_ID")?,
                redirect_uri: required("OAUTH_REDIRECT_URI")?,
                required_scope: env_or("OIDC_REQUIRED_SCOPE", "learning:access"),
            }),
            other => bail!(
                "unsupported AUTH_MODE {other:?}; expected oidc, bearer, embedded_oauth, or disabled"
            ),
        };

        Ok(Self {
            bind_addr,
            database_path,
            database_timeout: Duration::from_secs(database_timeout_seconds),
            max_request_bytes,
            auth,
        })
    }
}

fn required(name: &str) -> Result<String> {
    env::var(name).with_context(|| format!("required environment variable {name} is not set"))
}

fn env_or(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_owned())
}

fn parse_or<T>(name: &str, default: T) -> Result<T>
where
    T: FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    match env::var(name) {
        Ok(value) => value
            .parse::<T>()
            .with_context(|| format!("{name} has an invalid value")),
        Err(_) => Ok(default),
    }
}

fn trim_trailing_slash(mut value: String) -> String {
    while value.ends_with('/') {
        value.pop();
    }
    value
}

#[cfg(test)]
mod tests {
    use super::trim_trailing_slash;

    #[test]
    fn trims_all_trailing_slashes() {
        assert_eq!(
            trim_trailing_slash("https://example.com///".into()),
            "https://example.com"
        );
    }
}
