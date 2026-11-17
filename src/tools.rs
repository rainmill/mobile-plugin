//! The screens the app form opens: its build files, and a new upload token.

use std::sync::Arc;

use laterite_admin::axum::extract::{DefaultBodyLimit, Path};
use laterite_admin::axum::http::{header, HeaderMap, StatusCode};
use laterite_admin::axum::response::{IntoResponse, Response};
use laterite_admin::axum::routing::{get, post};
use laterite_admin::axum::{Extension, Router};
use laterite_admin::html::Markup;
use laterite_admin::routes::{RouteCtx, Screen};
use laterite_admin::{
    AuthenticatedUser, FlashLevel, PageHead, SessionHandle, Shell, VerifiedUpload,
};
use laterite_core::{t, Text};
use laterite_media::{blob_path, DiskReg, StorageDriver};

use crate::files::{self, FileError};

#[derive(Clone)]
struct Cx {
    ctx: RouteCtx,
    disk: Arc<dyn StorageDriver>,
}

impl Cx {
    fn app(&self, id: i64) -> String {
        format!("{}/rainmill/mobile/apps/{id}/edit", self.ctx.admin_path())
    }
    fn files(&self, id: i64) -> String {
        self.ctx.url(&format!("apps/{id}/files"))
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn size_text(bytes: i64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.0} KB", (bytes as f64 / 1024.0).max(1.0))
    }
}

fn redirect(to: String, headers: &HeaderMap) -> Response {
    if headers.contains_key("hx-request") {
        ([("HX-Redirect", to)], StatusCode::NO_CONTENT).into_response()
    } else {
        (StatusCode::SEE_OTHER, [(header::LOCATION, to)]).into_response()
    }
}

fn missing() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

fn failed(e: impl std::fmt::Display) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response()
}

/// A new upload token, shown this once.
async fn token(cx: &Cx, id: i64, shell: Shell, user: AuthenticatedUser) -> Response {
    let app = match files::app(cx.ctx.db(), id).await {
        Ok(Some(app)) => app,
        Ok(None) => return missing(),
        Err(e) => return failed(e),
    };
    let (token, hash) = files::new_token();
    if let Err(e) = files::set_token(cx.ctx.db(), id, &hash).await {
        return failed(e);
    }
    cx.ctx
        .audit(
            &user,
            "rainmill.mobile.app.upload_token",
            Some("mobile_apps"),
            Some(&id.to_string()),
            Some(&app.name),
            None,
        )
        .await;
    let body = format!(
        "<div class=\"lat-card\"><p>{}</p><p><input class=\"lat-input\" readonly value=\"{}\" onfocus=\"this.select()\" style=\"width:100%;font-family:monospace\"></p><p class=\"lat-note\">{}</p></div>",
        escape(&shell.t("The upload token for this app. It is shown this once: copy it into your pipeline's secrets now. A token made earlier has stopped working.")),
        escape(&token),
        escape(&shell.t("A pipeline sends it as \"Authorization: Bearer\" with each build file. Making another replaces this one.")),
    );
    shell.page_with(
        PageHead::new(t!("Upload token")).back(Text::dynamic(app.name), cx.app(id)),
        Markup::raw_unchecked(body),
    )
}

/// An app's releases with the build files each holds, and a form to add one.
async fn files_page(cx: &Cx, id: i64, shell: Shell) -> Response {
    let db = cx.ctx.db();
    let app = match files::app(db, id).await {
        Ok(Some(app)) => app,
        Ok(None) => return missing(),
        Err(e) => return failed(e),
    };
    let releases = match files::releases(db, id).await {
        Ok(r) => r,
        Err(e) => return failed(e),
    };
    let mut body = format!(
        "<p class=\"lat-note\">{}</p>",
        escape(&shell.t(if app.files_in_library {
            "This app keeps its build files in the media library, on the private disk."
        } else {
            "This app's build files are attached to their release alone, and are not in the media library."
        }))
    );
    if releases.is_empty() {
        body.push_str(&format!(
            "<p class=\"lat-note\">{}</p>",
            escape(&shell.t("No releases yet. Add a release on the app's page, or let a pipeline upload a build: it makes the release."))
        ));
    } else {
        let mut options = String::new();
        let mut rows = String::new();
        for (release, platform, version, build, track, state) in &releases {
            options.push_str(&format!(
                "<option value=\"{release}\">{} {} ({build}), {}</option>",
                escape(platform),
                escape(version),
                escape(track)
            ));
            let held = files::of_release(db, *release).await.unwrap_or_default();
            let list = if held.is_empty() {
                format!(
                    "<span class=\"lat-muted\">{}</span>",
                    escape(&shell.t("No files"))
                )
            } else {
                held.iter()
                    .map(|f| {
                        format!(
                            "<a href=\"{}\">{}</a> <span class=\"lat-muted\">{}, {}…</span> <form method=\"post\" action=\"{}\" style=\"display:inline\"><input type=\"hidden\" name=\"_csrf\" value=\"{}\"><button class=\"lat-btn lat-btn--ghost\" type=\"submit\" data-confirm=\"{}\">{}</button></form>",
                            escape(&cx.ctx.url(&format!("files/{release}/{}", f.id))),
                            escape(f.display_name()),
                            escape(&size_text(f.size)),
                            escape(&f.hash.chars().take(12).collect::<String>()),
                            escape(&cx.ctx.url(&format!("files/{release}/{}/remove", f.id))),
                            escape(shell.csrf_token()),
                            escape(&shell.t("Take this file off the release?")),
                            escape(&shell.t("Remove")),
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("<br>")
            };
            rows.push_str(&format!(
                "<tr><td>{}</td><td><strong>{}</strong> ({build})</td><td>{}</td><td>{}</td><td>{list}</td></tr>",
                escape(platform),
                escape(version),
                escape(track),
                escape(state),
            ));
        }
        body.push_str(&format!(
            "<div class=\"lat-tablewrap\"><table class=\"lat-table\"><thead><tr><th>{}</th><th>{}</th><th>{}</th><th>{}</th><th>{}</th></tr></thead><tbody>{rows}</tbody></table></div>",
            escape(&shell.t("Platform")),
            escape(&shell.t("Version")),
            escape(&shell.t("Track")),
            escape(&shell.t("State")),
            escape(&shell.t("Build files")),
        ));
        body.push_str(&format!(
            "<form method=\"post\" action=\"{}\" enctype=\"multipart/form-data\" class=\"lat-card\" style=\"margin-top:20px\"><input type=\"hidden\" name=\"_csrf\" value=\"{}\"><h3 class=\"lat-field__heading\">{}</h3><p><select class=\"lat-input\" name=\"release\">{options}</select></p><p><input type=\"file\" name=\"file\" accept=\".aab,.apk,.ipa,.zip\" required></p><p class=\"lat-note\">{}</p><div class=\"lat-actions\"><button class=\"lat-btn lat-btn--primary\" type=\"submit\">{}</button></div></form>",
            escape(&cx.files(id)),
            escape(shell.csrf_token()),
            escape(&shell.t("Add a build file")),
            escape(&shell.t("An AAB, APK, IPA or ZIP of up to 256 MB.")),
            escape(&shell.t("Upload")),
        ));
    }
    shell.page_with(
        PageHead::new(t!("Build files")).back(Text::dynamic(app.name), cx.app(id)),
        Markup::raw_unchecked(body),
    )
}

async fn upload(
    cx: &Cx,
    id: i64,
    session: SessionHandle,
    user: AuthenticatedUser,
    mut upload: VerifiedUpload,
) -> Response {
    let db = cx.ctx.db();
    let back = cx.files(id);
    let to = |back: &str| {
        (
            StatusCode::SEE_OTHER,
            [(header::LOCATION, back.to_string())],
        )
            .into_response()
    };
    let app = match files::app(db, id).await {
        Ok(Some(app)) => app,
        Ok(None) => return missing(),
        Err(e) => return failed(e),
    };
    let mut release: Option<i64> = None;
    let mut file: Option<(String, Vec<u8>)> = None;
    while let Ok(Some(mut field)) = upload.next_field().await {
        match field.name().unwrap_or_default() {
            "release" => release = field.text().await.ok().and_then(|t| t.trim().parse().ok()),
            "file" => {
                let name = field.file_name().unwrap_or_default().to_string();
                let mut bytes = Vec::new();
                while let Ok(Some(chunk)) = field.chunk().await {
                    if bytes.len() as u64 + chunk.len() as u64 > files::LIMIT {
                        session.push_flash(
                            FlashLevel::Error,
                            Text::dynamic(FileError::Size.to_string()),
                        );
                        return to(&back);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                file = Some((name, bytes));
            }
            _ => {}
        }
    }
    // The release must be one of this app's.
    let own = files::releases(db, id).await.unwrap_or_default();
    let (Some(release), Some((name, bytes))) =
        (release.filter(|r| own.iter().any(|o| o.0 == *r)), file)
    else {
        session.push_flash(FlashLevel::Error, t!("Choose a release and a file."));
        return to(&back);
    };
    match files::add(
        db,
        cx.disk.as_ref(),
        &app,
        release,
        &name,
        bytes,
        Some(user.user.id),
    )
    .await
    {
        Ok(stored) => {
            cx.ctx
                .audit(
                    &user,
                    "rainmill.mobile.release.file_add",
                    Some("mobile_releases"),
                    Some(&release.to_string()),
                    Some(&name),
                    Some(&stored.hash),
                )
                .await;
            session.push_flash(FlashLevel::Success, t!("The build file was added."));
        }
        Err(e) => session.push_flash(FlashLevel::Error, Text::dynamic(e.to_string())),
    }
    to(&back)
}

/// The app a release belongs to, when `media` is one of its build files.
async fn held(cx: &Cx, release: i64, media: i64) -> Option<laterite_media::MediaRecord> {
    files::of_release(cx.ctx.db(), release)
        .await
        .ok()?
        .into_iter()
        .find(|f| f.id == media)
}

async fn download(cx: &Cx, release: i64, media: i64) -> Response {
    let Some(record) = held(cx, release, media).await else {
        return missing();
    };
    use laterite_admin::axum::body::Body;
    match cx.disk.get(&blob_path(&record.hash)).await {
        Ok(stream) => {
            let name: String = record
                .display_name()
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || "._-".contains(*c))
                .collect();
            (
                [
                    (header::CONTENT_TYPE, "application/octet-stream".to_string()),
                    (
                        header::CONTENT_DISPOSITION,
                        format!("attachment; filename=\"{name}\""),
                    ),
                    (header::CACHE_CONTROL, "private, no-store".to_string()),
                ],
                Body::from_stream(stream),
            )
                .into_response()
        }
        Err(_) => missing(),
    }
}

async fn remove(
    cx: &Cx,
    release: i64,
    media: i64,
    session: SessionHandle,
    user: AuthenticatedUser,
    headers: HeaderMap,
) -> Response {
    let Some(record) = held(cx, release, media).await else {
        return missing();
    };
    let app_id = files_app_of(cx, release).await;
    match files::remove(cx.ctx.db(), release, media).await {
        Ok(_) => {
            cx.ctx
                .audit(
                    &user,
                    "rainmill.mobile.release.file_remove",
                    Some("mobile_releases"),
                    Some(&release.to_string()),
                    Some(record.display_name()),
                    Some(&record.hash),
                )
                .await;
            session.push_flash(FlashLevel::Success, t!("The build file was removed."));
        }
        Err(e) => session.push_flash(FlashLevel::Error, Text::dynamic(e.to_string())),
    }
    match app_id {
        Some(id) => redirect(cx.files(id), &headers),
        None => redirect(
            format!("{}/rainmill/mobile/apps", cx.ctx.admin_path()),
            &headers,
        ),
    }
}

async fn files_app_of(cx: &Cx, release: i64) -> Option<i64> {
    use laterite_core::query::{bind_values, build};
    use laterite_core::sea_query::{Alias, Expr, Query};
    use laterite_core::AnyRowExt;
    let db = cx.ctx.db();
    let (sql, values) = build(
        db.backend,
        Query::select()
            .column(Alias::new("app_id"))
            .from(Alias::new("mobile_releases"))
            .and_where(Expr::col(Alias::new("id")).eq(release))
            .to_owned(),
    );
    bind_values(laterite_core::sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await
        .ok()??
        .get_int("app_id")
        .ok()
}

pub(crate) struct Tools;

impl Screen for Tools {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        let cx = Cx {
            ctx: ctx.clone(),
            disk: files::disk(&ctx.contributions::<DiskReg>(), ctx.storage_dir()),
        };
        let (a, b, c, d, e) = (cx.clone(), cx.clone(), cx.clone(), cx.clone(), cx.clone());
        Router::new()
            .route(
                "/apps/{id}/token",
                post(
                    move |Path(id): Path<i64>,
                          Extension(shell): Extension<Shell>,
                          Extension(user): Extension<AuthenticatedUser>| {
                        let cx = a.clone();
                        async move { token(&cx, id, shell, user).await }
                    },
                ),
            )
            .route(
                "/apps/{id}/files",
                get(
                    move |Path(id): Path<i64>, Extension(shell): Extension<Shell>| {
                        let cx = b.clone();
                        async move { files_page(&cx, id, shell).await }
                    },
                )
                .post(
                    move |Path(id): Path<i64>,
                          Extension(session): Extension<SessionHandle>,
                          Extension(user): Extension<AuthenticatedUser>,
                          form: VerifiedUpload| {
                        let cx = c.clone();
                        async move { upload(&cx, id, session, user, form).await }
                    },
                ),
            )
            .route(
                "/files/{release}/{media}",
                get(move |Path((release, media)): Path<(i64, i64)>| {
                    let cx = d.clone();
                    async move { download(&cx, release, media).await }
                }),
            )
            .route(
                "/files/{release}/{media}/remove",
                post(
                    move |Path((release, media)): Path<(i64, i64)>,
                          Extension(session): Extension<SessionHandle>,
                          Extension(user): Extension<AuthenticatedUser>,
                          headers: HeaderMap| {
                        let cx = e.clone();
                        async move { remove(&cx, release, media, session, user, headers).await }
                    },
                ),
            )
            .layer(DefaultBodyLimit::max(
                usize::try_from(files::LIMIT).unwrap_or(usize::MAX) + 1024 * 1024,
            ))
    }
}
