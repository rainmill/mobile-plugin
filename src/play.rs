//! Google Play's side: a service account's access token and the Developer
//! API calls the preflight makes.

use std::time::Duration;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use laterite_core::serde_json::{self, json, Value};
use rsa::pkcs1v15::SigningKey;
use rsa::pkcs8::DecodePrivateKey;
use rsa::sha2::Sha256;
use rsa::signature::{SignatureEncoding, Signer};
use rsa::RsaPrivateKey;
use serde::Deserialize;

const SCOPE: &str = "https://www.googleapis.com/auth/androidpublisher";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://androidpublisher.googleapis.com/androidpublisher/v3/applications";

/// What went wrong talking to Google, and what to do about it.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct PlayError(pub String);

/// The parts of a service account key file the plugin reads.
#[derive(Deserialize)]
pub struct ServiceAccount {
    /// The account's email, which Play Console invites as a user.
    pub client_email: String,
    private_key: String,
    #[serde(default)]
    token_uri: Option<String>,
}

impl ServiceAccount {
    /// Reads a key file Google Cloud downloaded.
    pub fn parse(json: &[u8]) -> Result<Self, PlayError> {
        let account: Self = serde_json::from_slice(json)
            .map_err(|_| PlayError("the key file is not a Google service account key".into()))?;
        if account.client_email.is_empty() || !account.private_key.contains("PRIVATE KEY") {
            return Err(PlayError(
                "the key file has no service account email or private key".into(),
            ));
        }
        Ok(account)
    }

    fn assertion(&self, now: i64) -> Result<String, PlayError> {
        let key = RsaPrivateKey::from_pkcs8_pem(&self.private_key)
            .map_err(|_| PlayError("the key file's private key cannot be read".into()))?;
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = URL_SAFE_NO_PAD.encode(
            json!({
                "iss": self.client_email,
                "scope": SCOPE,
                "aud": self.token_uri.as_deref().unwrap_or(TOKEN_URL),
                "iat": now,
                "exp": now + 600,
            })
            .to_string(),
        );
        let signing = format!("{header}.{claims}");
        let signature = SigningKey::<Sha256>::new(key).sign(signing.as_bytes());
        Ok(format!(
            "{signing}.{}",
            URL_SAFE_NO_PAD.encode(signature.to_bytes())
        ))
    }
}

/// What a refusal from the Developer API means, in the developer's terms.
pub fn explain(status: u16, message: &str, account: &str) -> String {
    let lower = message.to_ascii_lowercase();
    if lower.contains("has not been used in project") || lower.contains("it is disabled") {
        "The Google Play Android Developer API is not enabled on the service account's Google Cloud project. In Google Cloud Console open APIs and Services, Library, find it and press Enable, then wait a few minutes.".into()
    } else if status == 401 || status == 403 {
        format!("Google Play refused {account} for this app. In Play Console open Users and permissions, invite that address, add this app, and grant \"View app information\" and \"Release to production\".")
    } else if status == 404 {
        "Google Play has no app with this package name under the account the service account was invited to. Create the app in Play Console first, and check the package name.".into()
    } else {
        format!("Google Play answered {status}: {message}")
    }
}

/// A client for one app as one service account.
pub struct Play {
    account: ServiceAccount,
    package: String,
    http: reqwest::Client,
}

impl Play {
    /// A client for the app `package` as `account`.
    pub fn new(account: ServiceAccount, package: &str) -> Result<Self, PlayError> {
        let package = package.trim().to_string();
        let ok = package.contains('.')
            && package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
        if !ok {
            return Err(PlayError(
                "the package name looks like com.acme.app: letters, digits, dots and underscores"
                    .into(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|e| PlayError(e.to_string()))?;
        Ok(Self {
            account,
            package,
            http,
        })
    }

    /// The service account's email.
    pub fn account(&self) -> &str {
        &self.account.client_email
    }

    /// An access token for the Developer API.
    pub async fn token(&self) -> Result<String, PlayError> {
        let assertion = self.account.assertion(chrono::Utc::now().timestamp())?;
        let response = self
            .http
            .post(self.account.token_uri.as_deref().unwrap_or(TOKEN_URL))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", &assertion),
            ])
            .send()
            .await
            .map_err(|e| PlayError(format!("Google could not be reached: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        match body["access_token"].as_str() {
            Some(token) => Ok(token.to_string()),
            None => Err(PlayError(format!(
                "Google refused the key ({status}): {}. A deleted key is the usual cause: make a new one under the service account's Keys.",
                body["error_description"]
                    .as_str()
                    .or(body["error"].as_str())
                    .unwrap_or("no reason given")
            ))),
        }
    }

    async fn call(
        &self,
        token: &str,
        method: reqwest::Method,
        path: &str,
    ) -> Result<Value, PlayError> {
        let response = self
            .http
            .request(method, format!("{API}/{}{path}", self.package))
            .bearer_auth(token)
            .header("content-length", "0")
            .send()
            .await
            .map_err(|e| PlayError(format!("Google Play could not be reached: {e}")))?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() {
            Ok(body)
        } else {
            Err(PlayError(explain(
                status.as_u16(),
                body["error"]["message"].as_str().unwrap_or_default(),
                self.account(),
            )))
        }
    }

    /// Opens an edit, reads each track's releases through it, and discards
    /// it. Nothing in Play changes: an edit is a draft until committed.
    pub async fn tracks(&self, token: &str) -> Result<Vec<Track>, PlayError> {
        let edit = self.call(token, reqwest::Method::POST, "/edits").await?;
        let id = edit["id"].as_str().unwrap_or_default().to_string();
        let listed = self
            .call(token, reqwest::Method::GET, &format!("/edits/{id}/tracks"))
            .await;
        // Discarded whether or not the read worked.
        let _ = self
            .call(token, reqwest::Method::DELETE, &format!("/edits/{id}"))
            .await;
        Ok(tracks_of(&listed?))
    }
}

/// A track and the newest build on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// `internal`, `alpha`, `beta`, `production` or a custom name.
    pub name: String,
    /// The highest version code among its releases.
    pub build: Option<i64>,
    /// That release's status (`draft`, `inProgress`, `completed`, `halted`).
    pub status: String,
}

/// The tracks of a `tracks.list` answer.
pub fn tracks_of(body: &Value) -> Vec<Track> {
    body["tracks"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|track| {
            let newest = track["releases"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|release| {
                    let build = release["versionCodes"]
                        .as_array()?
                        .iter()
                        .filter_map(|c| c.as_str()?.parse::<i64>().ok())
                        .max()?;
                    Some((
                        build,
                        release["status"].as_str().unwrap_or_default().to_string(),
                    ))
                })
                .max_by_key(|(build, _)| *build);
            Track {
                name: track["track"].as_str().unwrap_or_default().to_string(),
                build: newest.as_ref().map(|n| n.0),
                status: newest.map(|n| n.1).unwrap_or_default(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_says_what_to_do() {
        let disabled = explain(403, "Google Play Android Developer API has not been used in project 123 before or it is disabled.", "ci@acme.iam.gserviceaccount.com");
        assert!(disabled.contains("press Enable"));
        let uninvited = explain(
            403,
            "The caller does not have permission",
            "ci@acme.iam.gserviceaccount.com",
        );
        assert!(uninvited.contains("invite that address") && uninvited.contains("ci@acme"));
        assert!(explain(404, "Package not found: com.acme.app.", "x").contains("Create the app"));
        assert!(explain(500, "backend error", "x").contains("500"));
    }

    #[test]
    fn tracks_report_their_newest_build() {
        let body = json!({ "tracks": [
            { "track": "production", "releases": [
                { "versionCodes": ["401"], "status": "completed" },
                { "versionCodes": ["404", "403"], "status": "draft" } ] },
            { "track": "internal", "releases": [{ "name": "empty" }] },
            { "track": "beta" } ] });
        assert_eq!(
            tracks_of(&body),
            vec![
                Track {
                    name: "production".into(),
                    build: Some(404),
                    status: "draft".into()
                },
                Track {
                    name: "internal".into(),
                    build: None,
                    status: String::new()
                },
                Track {
                    name: "beta".into(),
                    build: None,
                    status: String::new()
                },
            ]
        );
    }

    #[test]
    fn a_key_file_and_a_package_name_are_checked() {
        assert!(ServiceAccount::parse(b"{}").is_err());
        assert!(ServiceAccount::parse(br#"{"client_email":"a@b","private_key":"nope"}"#).is_err());
        let account = || {
            ServiceAccount::parse(
                br#"{"client_email":"a@b","private_key":"-----BEGIN PRIVATE KEY-----"}"#,
            )
            .unwrap()
        };
        assert!(Play::new(account(), "com.acme.app").is_ok());
        assert!(Play::new(account(), "acme").is_err());
        assert!(Play::new(account(), "com.acme/app").is_err());
    }
}
