//! Reading apps, platforms, policies and releases, and recording devices.

use laterite_core::query::{bind_values, build};
use laterite_core::sea_query;
use laterite_core::sqlx;
use laterite_core::{AnyRowExt, Db};
use sea_query::{Alias, Expr, Order, Query};

use crate::policy::{Floors, Kind, Policy};

fn col(name: &str) -> Expr {
    Expr::col(Alias::new(name))
}

/// A registered platform of an app: one build variant's identifier.
#[derive(Clone, Debug)]
pub struct Platform {
    /// The row id.
    pub id: i64,
    /// The app it belongs to.
    pub app_id: i64,
    /// `android`, `ios` or `web`.
    pub kind: String,
    /// The environment its variant runs against.
    pub environment: String,
    /// Package name, bundle id or hostname.
    pub identifier: String,
    /// Where the app is updated from.
    pub store_url: Option<String>,
    /// Apple's team id.
    pub team_id: Option<String>,
    /// SHA-256 signing fingerprints.
    pub fingerprints: Vec<String>,
    /// Paths the app opens from links.
    pub link_paths: Vec<String>,
    /// Builds below this must update.
    pub min_build: Option<i64>,
    /// Builds below this are asked to.
    pub recommended_build: Option<i64>,
}

fn lines(text: Option<String>) -> Vec<String> {
    text.unwrap_or_default()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect()
}

const PLATFORM_COLUMNS: [&str; 11] = [
    "id",
    "app_id",
    "kind",
    "environment",
    "identifier",
    "store_url",
    "team_id",
    "fingerprints",
    "link_paths",
    "min_build",
    "recommended_build",
];

fn platform_of(row: &sqlx::any::AnyRow) -> Result<Platform, sqlx::Error> {
    Ok(Platform {
        id: row.get_int("id")?,
        app_id: row.get_int("app_id")?,
        kind: row.get_text("kind")?,
        environment: row.get_text("environment")?,
        identifier: row.get_text("identifier")?,
        store_url: row.get_text_opt("store_url")?.filter(|s| !s.is_empty()),
        team_id: row.get_text_opt("team_id")?.filter(|s| !s.is_empty()),
        fingerprints: lines(row.get_text_opt("fingerprints")?),
        link_paths: lines(row.get_text_opt("link_paths")?),
        min_build: row.get_int_opt("min_build")?,
        recommended_build: row.get_int_opt("recommended_build")?,
    })
}

/// The platform of the app `slug` registered as `kind` under `identifier`.
pub async fn platform(
    db: &Db,
    slug: &str,
    kind: &str,
    identifier: &str,
) -> Result<Option<Platform>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(PLATFORM_COLUMNS.map(|c| (Alias::new("mobile_platforms"), Alias::new(c))))
            .from(Alias::new("mobile_platforms"))
            .inner_join(
                Alias::new("mobile_apps"),
                Expr::col((Alias::new("mobile_apps"), Alias::new("id")))
                    .equals((Alias::new("mobile_platforms"), Alias::new("app_id"))),
            )
            .and_where(Expr::col((Alias::new("mobile_apps"), Alias::new("slug"))).eq(slug))
            .and_where(Expr::col((Alias::new("mobile_platforms"), Alias::new("kind"))).eq(kind))
            .and_where(
                Expr::col((Alias::new("mobile_platforms"), Alias::new("identifier")))
                    .eq(identifier),
            )
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    row.as_ref().map(platform_of).transpose()
}

/// Every registered platform of `kind`, across apps.
pub async fn platforms_of_kind(db: &Db, kind: &str) -> Result<Vec<Platform>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(PLATFORM_COLUMNS.map(Alias::new))
            .from(Alias::new("mobile_platforms"))
            .and_where(col("kind").eq(kind))
            .order_by(Alias::new("id"), Order::Asc)
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    rows.iter().map(platform_of).collect()
}

/// The enabled policies of an app.
pub async fn policies(db: &Db, app_id: i64) -> Result<Vec<Policy>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(
                [
                    "id",
                    "kind",
                    "key",
                    "value",
                    "message",
                    "platform",
                    "environment",
                    "min_build",
                    "max_build",
                    "locales",
                    "percent",
                    "starts_at",
                    "ends_at",
                ]
                .map(Alias::new),
            )
            .from(Alias::new("mobile_policies"))
            .and_where(col("app_id").eq(app_id))
            .and_where(col("enabled").eq(1))
            .order_by(Alias::new("id"), Order::Asc)
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let Some(kind) = Kind::parse(&row.get_text("kind")?) else {
            continue;
        };
        let some = |name: &str| -> Result<Option<String>, sqlx::Error> {
            Ok(row.get_text_opt(name)?.filter(|s| !s.trim().is_empty()))
        };
        out.push(Policy {
            id: row.get_int("id")?,
            kind,
            key: row.get_text("key")?,
            value: row.get_text_opt("value")?.unwrap_or_default(),
            message: row.get_text_opt("message")?.unwrap_or_default(),
            platform: some("platform")?,
            environment: some("environment")?,
            min_build: row.get_int_opt("min_build")?,
            max_build: row.get_int_opt("max_build")?,
            locales: row
                .get_text_opt("locales")?
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_string)
                .collect(),
            percent: row.get_int_opt("percent")?,
            starts_at: some("starts_at")?,
            ends_at: some("ends_at")?,
        });
    }
    Ok(out)
}

/// A platform's update floors with its newest released production build.
pub async fn floors(db: &Db, platform: &Platform) -> Result<Floors, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["build", "version"].map(Alias::new))
            .from(Alias::new("mobile_releases"))
            .and_where(col("app_id").eq(platform.app_id))
            .and_where(col("platform").eq(platform.kind.as_str()))
            .and_where(col("state").eq("released"))
            .and_where(col("track").eq("production"))
            .order_by(Alias::new("build"), Order::Desc)
            .limit(1)
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    let latest = match row {
        Some(row) => Some((row.get_int("build")?, row.get_text("version")?)),
        None => None,
    };
    Ok(Floors {
        min_build: platform.min_build,
        recommended_build: platform.recommended_build,
        latest,
        store_url: platform.store_url.clone(),
    })
}

/// What an install reports about itself when it checks in.
#[derive(Clone, Debug, Default)]
pub struct Seen {
    /// The id the install made once and keeps.
    pub install_id: String,
    /// The version people see.
    pub app_version: String,
    /// The build number.
    pub app_build: i64,
    /// The operating system's version.
    pub os_version: String,
    /// The device model, coarse.
    pub device_model: String,
    /// The locale.
    pub locale: String,
    /// The time zone.
    pub timezone: String,
}

fn bounded(text: &str) -> Option<String> {
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(120).collect())
}

/// Records that an install checked in: a new device, or the same one seen again.
pub async fn seen(db: &Db, platform: &Platform, seen: &Seen, now: &str) -> Result<(), sqlx::Error> {
    let values = |with_first: bool| {
        let mut v: Vec<(Alias, sea_query::SimpleExpr)> = vec![
            (Alias::new("platform_id"), platform.id.into()),
            (Alias::new("platform"), platform.kind.as_str().into()),
            (
                Alias::new("environment"),
                platform.environment.as_str().into(),
            ),
            (Alias::new("app_version"), bounded(&seen.app_version).into()),
            (Alias::new("app_build"), seen.app_build.into()),
            (Alias::new("os_version"), bounded(&seen.os_version).into()),
            (
                Alias::new("device_model"),
                bounded(&seen.device_model).into(),
            ),
            (Alias::new("locale"), bounded(&seen.locale).into()),
            (Alias::new("timezone"), bounded(&seen.timezone).into()),
            (Alias::new("last_seen_at"), now.into()),
        ];
        if with_first {
            v.push((Alias::new("app_id"), platform.app_id.into()));
            v.push((Alias::new("install_id"), seen.install_id.as_str().into()));
            v.push((Alias::new("first_seen_at"), now.into()));
        }
        v
    };
    let update = || {
        build(
            db.backend,
            Query::update()
                .table(Alias::new("mobile_devices"))
                .values(values(false))
                .and_where(col("app_id").eq(platform.app_id))
                .and_where(col("install_id").eq(seen.install_id.as_str()))
                .to_owned(),
        )
    };
    let (sql, bound) = update();
    let done = bind_values(sqlx::query(&sql), bound)
        .execute(&db.pool)
        .await?;
    if done.rows_affected() > 0 {
        return Ok(());
    }
    let (columns, exprs): (Vec<_>, Vec<_>) = values(true).into_iter().unzip();
    let (sql, bound) = build(
        db.backend,
        Query::insert()
            .into_table(Alias::new("mobile_devices"))
            .columns(columns)
            .values_panic(exprs)
            .to_owned(),
    );
    match bind_values(sqlx::query(&sql), bound)
        .execute(&db.pool)
        .await
    {
        Ok(_) => Ok(()),
        // Two check-ins of a new install at once: the other one made the row.
        Err(_) => {
            let (sql, bound) = update();
            bind_values(sqlx::query(&sql), bound)
                .execute(&db.pool)
                .await
                .map(|_| ())
        }
    }
}
