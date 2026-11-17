//! A release's build files: stored as media records linked to the release,
//! in the media library or attached to the release alone, as the app chooses.

use std::sync::Arc;

use laterite_core::query::{bind_values, build, insert_returning_id};
use laterite_core::sea_query::{Alias, Expr, Order, Query};
use laterite_core::{sqlx, AnyRowExt, Db};
use laterite_media::ingest::{ingest, stream_of};
use laterite_media::record::{self, MediaRecord, NewMedia};
use laterite_media::{link, DiskReg, LocalDisk, Owner, StorageDriver};

/// The disk build files are written to.
pub const DISK: &str = "private";
/// The entity a build file is linked to.
pub const OWNER: &str = "mobile_releases";
/// The field of a release that holds its build files.
pub const FIELD: &str = "files";
/// The largest build file taken, in bytes.
pub const LIMIT: u64 = 256 * 1024 * 1024;
/// The file types taken.
pub const EXTENSIONS: &[&str] = &["aab", "apk", "ipa", "zip"];

/// What went wrong storing a build file.
#[derive(Debug, thiserror::Error)]
pub enum FileError {
    /// The request named no app, or a wrong token.
    #[error("the app or its upload token is not recognised")]
    Refused,
    /// The file is of a type that is not a build.
    #[error("a build file is one of: aab, apk, ipa, zip")]
    Kind,
    /// The file is empty or over [`LIMIT`].
    #[error("the file is empty or larger than 256 MB")]
    Size,
    /// A value the request must carry is missing or malformed.
    #[error("{0}")]
    Input(&'static str),
    /// The file or its record could not be written.
    #[error("the file could not be stored: {0}")]
    Store(String),
}

impl From<sqlx::Error> for FileError {
    fn from(e: sqlx::Error) -> Self {
        Self::Store(e.to_string())
    }
}

/// The private disk: the one registered under [`DISK`], or a folder of the
/// storage directory.
pub fn disk(registered: &[&DiskReg], storage_dir: &str) -> Arc<dyn StorageDriver> {
    match registered.iter().find(|d| d.name == DISK) {
        Some(d) => Arc::clone(&d.driver),
        None => Arc::new(LocalDisk::new(format!("{storage_dir}/{DISK}"))),
    }
}

/// A new upload token and the hash to keep of it.
pub fn new_token() -> (String, String) {
    use rand::RngCore;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let token: String = format!(
        "latm_{}",
        bytes.iter().map(|b| format!("{b:02x}")).collect::<String>()
    );
    let hash = hash_of(&token);
    (token, hash)
}

/// The hash kept of a token.
pub fn hash_of(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}

/// An app as an upload needs it.
#[derive(Debug, Clone)]
pub struct App {
    /// The row id.
    pub id: i64,
    /// Its name.
    pub name: String,
    /// Whether its build files are kept in the media library.
    pub files_in_library: bool,
    /// The hash of its upload token, if one was made.
    pub token_hash: Option<String>,
}

async fn app_where(
    db: &Db,
    column: &str,
    value: laterite_core::sea_query::Value,
) -> Result<Option<App>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["id", "name", "files_in_library", "upload_token_hash"].map(Alias::new))
            .from(Alias::new("mobile_apps"))
            .and_where(Expr::col(Alias::new(column)).eq(value))
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?;
    Ok(match row {
        Some(row) => Some(App {
            id: row.get_int("id")?,
            name: row.get_text("name")?,
            files_in_library: row.get_bool("files_in_library")?,
            token_hash: row
                .get_text_opt("upload_token_hash")?
                .filter(|h| !h.is_empty()),
        }),
        None => None,
    })
}

/// The app with this id.
pub async fn app(db: &Db, id: i64) -> Result<Option<App>, sqlx::Error> {
    app_where(db, "id", id.into()).await
}

/// The app `slug`, when `token` is its upload token.
pub async fn app_for_token(db: &Db, slug: &str, token: &str) -> Result<App, FileError> {
    let app = app_where(db, "slug", slug.into()).await?;
    let given = hash_of(token);
    match app {
        // Compared as hashes of equal length, every byte, whatever differs.
        Some(app)
            if app.token_hash.as_deref().is_some_and(|kept| {
                kept.len() == given.len()
                    && kept
                        .bytes()
                        .zip(given.bytes())
                        .fold(0u8, |d, (a, b)| d | (a ^ b))
                        == 0
            }) =>
        {
            Ok(app)
        }
        _ => Err(FileError::Refused),
    }
}

/// Keeps the hash of a new upload token for an app, replacing the last.
pub async fn set_token(db: &Db, app_id: i64, hash: &str) -> Result<(), sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::update()
            .table(Alias::new("mobile_apps"))
            .value(Alias::new("upload_token_hash"), hash)
            .and_where(Expr::col(Alias::new("id")).eq(app_id))
            .to_owned(),
    );
    bind_values(sqlx::query(&sql), values)
        .execute(&db.pool)
        .await?;
    Ok(())
}

/// What a build is a release of.
#[derive(Debug, Clone)]
pub struct Build {
    /// `android`, `ios` or `web`.
    pub platform: String,
    /// The version people see.
    pub version: String,
    /// The build number.
    pub build: i64,
    /// `internal`, `beta` or `production`.
    pub track: String,
    /// What changed, for a release this upload creates.
    pub notes: String,
}

impl Build {
    fn checked(&self) -> Result<(), FileError> {
        if !["android", "ios", "web"].contains(&self.platform.as_str()) {
            return Err(FileError::Input("platform is android, ios or web"));
        }
        if self.version.trim().is_empty() || self.version.len() > 40 {
            return Err(FileError::Input("version is required, such as 1.4.0"));
        }
        if self.build < 0 {
            return Err(FileError::Input("build is a number from 0"));
        }
        if !["internal", "beta", "production"].contains(&self.track.as_str()) {
            return Err(FileError::Input("track is internal, beta or production"));
        }
        Ok(())
    }
}

/// The release of `app` this build belongs to, made as a draft when new.
pub async fn release_for(db: &Db, app_id: i64, b: &Build, now: &str) -> Result<i64, FileError> {
    b.checked()?;
    let (sql, values) = build(
        db.backend,
        Query::select()
            .column(Alias::new("id"))
            .from(Alias::new("mobile_releases"))
            .and_where(Expr::col(Alias::new("app_id")).eq(app_id))
            .and_where(Expr::col(Alias::new("platform")).eq(b.platform.as_str()))
            .and_where(Expr::col(Alias::new("build")).eq(b.build))
            .and_where(Expr::col(Alias::new("track")).eq(b.track.as_str()))
            .order_by(Alias::new("id"), Order::Asc)
            .to_owned(),
    );
    if let Some(row) = bind_values(sqlx::query(&sql), values)
        .fetch_optional(&db.pool)
        .await?
    {
        return Ok(row.get_int("id")?);
    }
    let stmt = Query::insert()
        .into_table(Alias::new("mobile_releases"))
        .columns(
            [
                "app_id",
                "platform",
                "version",
                "build",
                "track",
                "state",
                "changelog",
                "created_at",
                "updated_at",
            ]
            .map(Alias::new),
        )
        .values_panic([
            app_id.into(),
            b.platform.as_str().into(),
            b.version.trim().into(),
            b.build.into(),
            b.track.as_str().into(),
            "draft".into(),
            b.notes.as_str().into(),
            now.into(),
            now.into(),
        ])
        .to_owned();
    Ok(insert_returning_id(db, stmt, Alias::new("id")).await?)
}

/// A stored build file.
#[derive(Debug, Clone, PartialEq)]
pub struct Stored {
    /// The release it was added to.
    pub release_id: i64,
    /// Its media record.
    pub media_id: i64,
    /// BLAKE3 of its bytes, hex.
    pub hash: String,
    /// Its size in bytes.
    pub size: u64,
}

fn owner(id: &str) -> Owner<'_> {
    Owner {
        owner_type: OWNER,
        owner_id: id,
        field: FIELD,
    }
}

/// Stores `bytes` as a build file of a release: a media record on the private
/// disk, in the library when the app keeps them there, linked to the release
/// after the files it already has.
pub async fn add(
    db: &Db,
    disk: &dyn StorageDriver,
    app: &App,
    release_id: i64,
    name: &str,
    bytes: Vec<u8>,
    created_by: Option<i64>,
) -> Result<Stored, FileError> {
    let name: String = name
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(160)
        .collect();
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if !extension.is_some_and(|e| EXTENSIONS.contains(&e.as_str())) {
        return Err(FileError::Kind);
    }
    if bytes.is_empty() || bytes.len() as u64 > LIMIT {
        return Err(FileError::Size);
    }
    let stored = ingest(disk, stream_of(bytes), LIMIT)
        .await
        .map_err(|e| FileError::Store(e.to_string()))?;
    let media_id = record::create(
        db,
        NewMedia {
            disk: DISK.to_string(),
            hash: stored.hash.clone(),
            size: stored.size as i64,
            content_type: stored.content_type,
            original_name: name,
            in_library: app.files_in_library,
            created_by,
        },
    )
    .await
    .map_err(|e| FileError::Store(e.to_string()))?;
    let id = release_id.to_string();
    let mut held: Vec<i64> = link::of_owner(db, owner(&id))
        .await
        .map_err(|e| FileError::Store(e.to_string()))?
        .into_iter()
        .map(|l| l.media_id)
        .collect();
    held.push(media_id);
    link::set(db, owner(&id), &held)
        .await
        .map_err(|e| FileError::Store(e.to_string()))?;
    Ok(Stored {
        release_id,
        media_id,
        hash: stored.hash,
        size: stored.size,
    })
}

/// The build files of a release, in the order they were added.
pub async fn of_release(db: &Db, release_id: i64) -> Result<Vec<MediaRecord>, FileError> {
    let id = release_id.to_string();
    let links = link::of_owner(db, owner(&id))
        .await
        .map_err(|e| FileError::Store(e.to_string()))?;
    let mut out = Vec::with_capacity(links.len());
    for l in links {
        if let Some(r) = record::find(db, l.media_id)
            .await
            .map_err(|e| FileError::Store(e.to_string()))?
        {
            out.push(r);
        }
    }
    Ok(out)
}

/// Takes a build file off its release. A file kept in the library stays
/// there; one attached to the release alone is deleted with its record.
pub async fn remove(db: &Db, release_id: i64, media_id: i64) -> Result<bool, FileError> {
    let id = release_id.to_string();
    let held: Vec<i64> = link::of_owner(db, owner(&id))
        .await
        .map_err(|e| FileError::Store(e.to_string()))?
        .into_iter()
        .map(|l| l.media_id)
        .collect();
    if !held.contains(&media_id) {
        return Ok(false);
    }
    let rest: Vec<i64> = held.into_iter().filter(|m| *m != media_id).collect();
    link::set(db, owner(&id), &rest)
        .await
        .map_err(|e| FileError::Store(e.to_string()))?;
    let record = record::find(db, media_id)
        .await
        .map_err(|e| FileError::Store(e.to_string()))?;
    if record.is_some_and(|r| !r.in_library) {
        record::delete(db, media_id)
            .await
            .map_err(|e| FileError::Store(e.to_string()))?;
    }
    Ok(true)
}

/// The releases of an app as `(id, platform, version, build, track, state)`,
/// newest build first.
pub async fn releases(
    db: &Db,
    app_id: i64,
) -> Result<Vec<(i64, String, String, i64, String, String)>, sqlx::Error> {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["id", "platform", "version", "build", "track", "state"].map(Alias::new))
            .from(Alias::new("mobile_releases"))
            .and_where(Expr::col(Alias::new("app_id")).eq(app_id))
            .order_by(Alias::new("build"), Order::Desc)
            .order_by(Alias::new("id"), Order::Desc)
            .to_owned(),
    );
    let rows = bind_values(sqlx::query(&sql), values)
        .fetch_all(&db.pool)
        .await?;
    rows.iter()
        .map(|r| {
            Ok((
                r.get_int("id")?,
                r.get_text("platform")?,
                r.get_text("version")?,
                r.get_int("build")?,
                r.get_text("track")?,
                r.get_text("state")?,
            ))
        })
        .collect()
}
