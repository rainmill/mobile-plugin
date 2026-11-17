//! The public endpoints: the check-in an app makes at startup, and the two
//! files that let links to the site open in the app.

use std::sync::Arc;

use laterite_admin::axum::extract::State;
use laterite_admin::axum::http::{header, StatusCode};
use laterite_admin::axum::response::{IntoResponse, Response};
use laterite_admin::axum::routing::{get, post};
use laterite_admin::axum::{Json, Router};
use laterite_admin::routes::{PublicRoute, RouteCtx};
use laterite_core::serde_json::{json, Value};
use laterite_core::Db;
use serde::Deserialize;

use crate::policy::{evaluate, Subject};
use crate::store::{self, Seen};

/// Where an app checks in.
pub const HELLO_PATH: &str = "/mobile/v1/hello";
/// Apple's file naming the apps that open this site's links.
pub const APPLE_PATH: &str = "/.well-known/apple-app-site-association";
/// Android's file naming the apps that open this site's links.
pub const ANDROID_PATH: &str = "/.well-known/assetlinks.json";

/// What an app sends when it checks in.
#[derive(Debug, Deserialize)]
pub struct Hello {
    /// The app's slug.
    pub app: String,
    /// `android`, `ios` or `web`.
    pub platform: String,
    /// The package name, bundle id or hostname this build carries.
    pub identifier: String,
    /// The id the install made once and keeps.
    pub install_id: String,
    /// The build number.
    pub build: i64,
    /// The version people see.
    #[serde(default)]
    pub version: String,
    /// The operating system's version.
    #[serde(default)]
    pub os_version: String,
    /// The device model.
    #[serde(default)]
    pub device_model: String,
    /// The locale (`en-IN`).
    #[serde(default)]
    pub locale: String,
    /// The time zone (`Asia/Kolkata`).
    #[serde(default)]
    pub timezone: String,
}

fn problem(status: StatusCode, kind: &str, detail: &str) -> Response {
    (
        status,
        [(header::CONTENT_TYPE, "application/problem+json")],
        json!({ "type": format!("urn:rainmill:mobile:{kind}"), "status": status.as_u16(), "detail": detail })
            .to_string(),
    )
        .into_response()
}

fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

async fn hello(State(db): State<Db>, Json(hello): Json<Hello>) -> Response {
    let install = hello.install_id.trim();
    if install.is_empty() || install.len() > 64 || hello.build < 0 {
        return problem(
            StatusCode::BAD_REQUEST,
            "bad-request",
            "install_id and build are required",
        );
    }
    let platform = match store::platform(
        &db,
        hello.app.trim(),
        hello.platform.trim(),
        hello.identifier.trim(),
    )
    .await
    {
        Ok(Some(p)) => p,
        Ok(None) => {
            return problem(
                StatusCode::FORBIDDEN,
                "unknown-app",
                "this app is not registered here",
            );
        }
        Err(_) => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "try again shortly",
            )
        }
    };
    let now = now();
    let (policies, floors) = match (
        store::policies(&db, platform.app_id).await,
        store::floors(&db, &platform).await,
    ) {
        (Ok(p), Ok(f)) => (p, f),
        _ => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                "try again shortly",
            )
        }
    };
    let subject = Subject {
        platform: platform.kind.clone(),
        environment: platform.environment.clone(),
        build: hello.build,
        locale: hello.locale.clone(),
        install_id: install.to_string(),
    };
    let verdict = evaluate(&policies, &floors, &subject, &now);
    // The verdict stands whether or not the device row could be written.
    let _ = store::seen(
        &db,
        &platform,
        &Seen {
            install_id: install.to_string(),
            app_version: hello.version,
            app_build: hello.build,
            os_version: hello.os_version,
            device_model: hello.device_model,
            locale: hello.locale,
            timezone: hello.timezone,
        },
        &now,
    )
    .await;
    let mut body = laterite_core::serde_json::to_value(&verdict).unwrap_or(Value::Null);
    if let Value::Object(map) = &mut body {
        map.insert("environment".into(), platform.environment.into());
        map.insert("checked_at".into(), now.into());
    }
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}

/// Apple's file: each iOS platform with a team id, and the paths it opens.
pub fn apple_file(platforms: &[store::Platform]) -> Value {
    let details: Vec<Value> = platforms
        .iter()
        .filter_map(|p| {
            let team = p.team_id.as_deref()?;
            let paths: Vec<Value> = if p.link_paths.is_empty() {
                vec![json!({ "/": "/*" })]
            } else {
                p.link_paths
                    .iter()
                    .map(|path| json!({ "/": path }))
                    .collect()
            };
            Some(json!({ "appIDs": [format!("{team}.{}", p.identifier)], "components": paths }))
        })
        .collect();
    json!({ "applinks": { "details": details } })
}

/// Android's file: each Android platform with a signing fingerprint.
pub fn android_file(platforms: &[store::Platform]) -> Value {
    Value::Array(
        platforms
            .iter()
            .filter(|p| !p.fingerprints.is_empty())
            .map(|p| {
                json!({
                    "relation": ["delegate_permission/common.handle_all_urls"],
                    "target": {
                        "namespace": "android_app",
                        "package_name": p.identifier,
                        "sha256_cert_fingerprints": p.fingerprints,
                    }
                })
            })
            .collect(),
    )
}

async fn link_file(db: Db, kind: &str) -> Response {
    match store::platforms_of_kind(&db, kind).await {
        Ok(platforms) => {
            let body = if kind == "ios" {
                apple_file(&platforms)
            } else {
                android_file(&platforms)
            };
            (
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "public, max-age=3600"),
                ],
                body.to_string(),
            )
                .into_response()
        }
        Err(_) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
    }
}

pub(crate) struct HelloRoute;

impl PublicRoute for HelloRoute {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        Router::new()
            .route("/", post(hello))
            .with_state(ctx.db().clone())
    }
}

pub(crate) struct LinkFile(pub &'static str);

impl PublicRoute for LinkFile {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        let kind = self.0;
        Router::new()
            .route("/", get(move |State(db): State<Db>| link_file(db, kind)))
            .with_state(ctx.db().clone())
    }
}

pub(crate) fn hello_route() -> Arc<HelloRoute> {
    Arc::new(HelloRoute)
}
