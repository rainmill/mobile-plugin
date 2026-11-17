//! Apps, their platforms, the policies that decide what an install is told,
//! the releases on record and the devices that check in.

use laterite_core::sea_query;
use laterite_core::strata::*;
use sea_query::Alias;

pub struct Migration;

#[async_trait(?Send)]
impl laterite_core::Migration for Migration {
    fn name(&self) -> &str {
        "0001_create_mobile"
    }
    async fn up(&self, s: &mut Schema<'_>) -> CoreResult<()> {
        let id = || {
            ColumnDef::new(Alias::new("id"))
                .big_integer()
                .not_null()
                .auto_increment()
                .primary_key()
                .to_owned()
        };
        let text = |name: &str| ColumnDef::new(Alias::new(name)).text().null().to_owned();
        let key = |name: &str| key_col(Alias::new(name)).null().to_owned();
        let number = |name: &str| ColumnDef::new(Alias::new(name)).big_integer().null().to_owned();
        let parent = |name: &str| ColumnDef::new(Alias::new(name)).big_integer().not_null().to_owned();
        s.exec(
            Table::create()
                .table(Alias::new("mobile_apps"))
                .col(id())
                .col(ColumnDef::new(Alias::new("name")).text().not_null())
                // The app's key in code and in the check-in request.
                .col(key_col(Alias::new("slug")).not_null().unique_key())
                .col(text("description"))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::create()
                .table(Alias::new("mobile_platforms"))
                .col(id())
                .col(parent("app_id"))
                // android, ios or web.
                .col(key_col(Alias::new("kind")).not_null())
                // The build variant's name (a product flavor, a build configuration).
                .col(text("variant"))
                // development, staging, production, or one the application adds.
                .col(key_col(Alias::new("environment")).not_null().default("production"))
                // Package name, bundle id or hostname; an install must name one.
                .col(key_col(Alias::new("identifier")).not_null())
                .col(text("store_url"))
                // Apple's numeric app id.
                .col(key("store_id"))
                // Apple's team id, for the app id a universal link names.
                .col(key("team_id"))
                // SHA-256 signing certificate fingerprints, one a line (Android).
                .col(text("fingerprints"))
                // Paths the app opens from links, one a line; every path when empty.
                .col(text("link_paths"))
                // Builds below this must update; below the next are asked to.
                .col(number("min_build"))
                .col(number("recommended_build"))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Index::create()
                .name("mobile_platforms_identifier")
                .table(Alias::new("mobile_platforms"))
                .col(Alias::new("kind"))
                .col(Alias::new("identifier"))
                .unique()
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::create()
                .table(Alias::new("mobile_policies"))
                .col(id())
                .col(parent("app_id"))
                // flag, maintenance, update or banner.
                .col(key_col(Alias::new("kind")).not_null())
                // A flag's or banner's name; free for the other kinds.
                .col(key_col(Alias::new("key")).not_null())
                // A flag's value, or soft or hard for an update.
                .col(text("value"))
                // What the person reads.
                .col(text("message"))
                .col(bool_col(Alias::new("enabled")).not_null().default(1))
                // Who it applies to; every install where a column is empty.
                .col(key("platform"))
                .col(key("environment"))
                .col(number("min_build"))
                .col(number("max_build"))
                // Locales, comma separated (en-IN, hi).
                .col(text("locales"))
                // A share of installs, 1 to 100.
                .col(number("percent"))
                // When it holds (UTC); always where empty.
                .col(text("starts_at"))
                .col(text("ends_at"))
                .col(text("note"))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::create()
                .table(Alias::new("mobile_releases"))
                .col(id())
                .col(parent("app_id"))
                .col(key_col(Alias::new("platform")).not_null())
                // The version people see (1.4.0) and the build number that orders releases.
                .col(key_col(Alias::new("version")).not_null())
                .col(ColumnDef::new(Alias::new("build")).big_integer().not_null())
                // internal, beta or production.
                .col(key_col(Alias::new("track")).not_null().default("production"))
                // draft, released or withdrawn.
                .col(key_col(Alias::new("state")).not_null().default("draft"))
                // What changed, in markdown.
                .col(text("changelog"))
                .col(text("released_at"))
                .col(text("created_at"))
                .col(text("updated_at"))
                .to_owned(),
        )
        .await?;
        s.exec(
            Table::create()
                .table(Alias::new("mobile_devices"))
                .col(id())
                .col(parent("app_id"))
                .col(parent("platform_id"))
                // The id an install makes once and keeps.
                .col(key_col(Alias::new("install_id")).not_null())
                .col(key_col(Alias::new("platform")).not_null())
                .col(key_col(Alias::new("environment")).not_null())
                .col(key("app_version"))
                .col(number("app_build"))
                .col(key("os_version"))
                .col(key("device_model"))
                .col(key("locale"))
                .col(key("timezone"))
                .col(ColumnDef::new(Alias::new("first_seen_at")).text().not_null())
                .col(ColumnDef::new(Alias::new("last_seen_at")).text().not_null())
                .to_owned(),
        )
        .await?;
        s.exec(
            Index::create()
                .name("mobile_devices_install")
                .table(Alias::new("mobile_devices"))
                .col(Alias::new("app_id"))
                .col(Alias::new("install_id"))
                .unique()
                .to_owned(),
        )
        .await
    }
}
