//! The store against a real database: a registered platform is found, an
//! unregistered one is refused, a check-in records one device, and the link
//! files name the registered apps.

use laterite_core::query::{bind_values, build, insert_returning_id};
use laterite_core::sea_query::{Alias, Query};
use laterite_core::testing::connect_test;
use laterite_core::{sqlx, AnyRowExt, Db};
use rainmill_mobile::api::{android_file, apple_file};
use rainmill_mobile::policy::{evaluate, Level, Subject};
use rainmill_mobile::store::{self, Seen};

async fn insert(
    db: &Db,
    table: &str,
    values: &[(&str, laterite_core::sea_query::SimpleExpr)],
) -> i64 {
    let stmt = Query::insert()
        .into_table(Alias::new(table))
        .columns(values.iter().map(|(c, _)| Alias::new(*c)))
        .values_panic(values.iter().map(|(_, v)| v.clone()))
        .to_owned();
    insert_returning_id(db, stmt, Alias::new("id"))
        .await
        .unwrap()
}

async fn count(db: &Db, table: &str) -> i64 {
    let (sql, values) = build(
        db.backend,
        Query::select()
            .expr_as(
                laterite_core::sea_query::Expr::cust("COUNT(*)"),
                Alias::new("n"),
            )
            .from(Alias::new(table))
            .to_owned(),
    );
    bind_values(sqlx::query(&sql), values)
        .fetch_one(&db.pool)
        .await
        .unwrap()
        .get_int("n")
        .unwrap()
}

async fn seeded() -> (Db, laterite_core::testing::TestGuard, i64) {
    let (db, guard) = connect_test(&[rainmill_mobile::module().migrations()]).await;
    let app = insert(
        &db,
        "mobile_apps",
        &[("name", "Acme".into()), ("slug", "acme".into())],
    )
    .await;
    insert(
        &db,
        "mobile_platforms",
        &[
            ("app_id", app.into()),
            ("kind", "android".into()),
            ("environment", "production".into()),
            ("identifier", "com.acme.app".into()),
            ("fingerprints", "AA:BB\n CC:DD \n".into()),
            ("min_build", 10.into()),
            ("recommended_build", 20.into()),
            ("store_url", "https://acme.com/get".into()),
        ],
    )
    .await;
    insert(
        &db,
        "mobile_platforms",
        &[
            ("app_id", app.into()),
            ("kind", "ios".into()),
            ("environment", "staging".into()),
            ("identifier", "com.acme.app.staging".into()),
            ("team_id", "TEAM123456".into()),
            ("link_paths", "/events/*".into()),
        ],
    )
    .await;
    (db, guard, app)
}

#[tokio::test]
async fn a_registered_build_gets_its_verdict_and_an_unregistered_one_is_unknown() {
    let (db, _guard, app) = seeded().await;
    assert!(store::platform(&db, "acme", "android", "com.other.app")
        .await
        .unwrap()
        .is_none());
    assert!(store::platform(&db, "other", "android", "com.acme.app")
        .await
        .unwrap()
        .is_none());
    let platform = store::platform(&db, "acme", "android", "com.acme.app")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(platform.fingerprints, vec!["AA:BB", "CC:DD"]);

    for (kind, key, value, enabled, environment) in [
        ("flag", "new_feed", "true", 1, None),
        ("flag", "off_flag", "true", 0, None),
        ("flag", "staging_only", "true", 1, Some("staging")),
        ("maintenance", "upgrade", "", 1, Some("staging")),
    ] {
        insert(
            &db,
            "mobile_policies",
            &[
                ("app_id", app.into()),
                ("kind", kind.into()),
                ("key", key.into()),
                ("value", value.into()),
                ("message", "Back soon.".into()),
                ("enabled", enabled.into()),
                ("environment", environment.map(str::to_string).into()),
            ],
        )
        .await;
    }
    for (build, state, track) in [
        (22, "released", "production"),
        (25, "draft", "production"),
        (24, "released", "beta"),
    ] {
        insert(
            &db,
            "mobile_releases",
            &[
                ("app_id", app.into()),
                ("platform", "android".into()),
                ("version", format!("1.{build}.0").into()),
                ("build", build.into()),
                ("track", track.into()),
                ("state", state.into()),
            ],
        )
        .await;
    }

    let policies = store::policies(&db, app).await.unwrap();
    assert_eq!(policies.len(), 3, "the policy turned off is left out");
    let floors = store::floors(&db, &platform).await.unwrap();
    assert_eq!(
        floors.latest,
        Some((22, "1.22.0".to_string())),
        "drafts and beta releases are not the latest"
    );

    let subject = Subject {
        platform: platform.kind.clone(),
        environment: platform.environment.clone(),
        build: 15,
        locale: "en-IN".into(),
        install_id: "install-1".into(),
    };
    let verdict = evaluate(&policies, &floors, &subject, "2026-11-18T10:00:00Z");
    assert_eq!(verdict.update.level, Level::Soft);
    assert_eq!(
        verdict.update.store_url.as_deref(),
        Some("https://acme.com/get")
    );
    assert!(verdict.maintenance.is_none(), "maintenance was for staging");
    assert_eq!(verdict.flags.keys().collect::<Vec<_>>(), ["new_feed"]);
}

#[tokio::test]
async fn an_install_is_one_device_however_often_it_checks_in() {
    let (db, _guard, _) = seeded().await;
    let platform = store::platform(&db, "acme", "android", "com.acme.app")
        .await
        .unwrap()
        .unwrap();
    let mut seen = Seen {
        install_id: "install-1".into(),
        app_version: "1.4.0".into(),
        app_build: 15,
        locale: "en-IN".into(),
        ..Seen::default()
    };
    store::seen(&db, &platform, &seen, "2026-11-18T10:00:00Z")
        .await
        .unwrap();
    seen.app_build = 22;
    store::seen(&db, &platform, &seen, "2026-11-19T10:00:00Z")
        .await
        .unwrap();
    seen.install_id = "install-2".into();
    store::seen(&db, &platform, &seen, "2026-11-19T11:00:00Z")
        .await
        .unwrap();
    assert_eq!(count(&db, "mobile_devices").await, 2);

    let (sql, values) = build(
        db.backend,
        Query::select()
            .columns(["app_build", "first_seen_at", "last_seen_at"].map(Alias::new))
            .from(Alias::new("mobile_devices"))
            .and_where(
                laterite_core::sea_query::Expr::col(Alias::new("install_id")).eq("install-1"),
            )
            .to_owned(),
    );
    let row = bind_values(sqlx::query(&sql), values)
        .fetch_one(&db.pool)
        .await
        .unwrap();
    assert_eq!(row.get_int("app_build").unwrap(), 22);
    assert_eq!(
        row.get_text("first_seen_at").unwrap(),
        "2026-11-18T10:00:00Z"
    );
    assert_eq!(
        row.get_text("last_seen_at").unwrap(),
        "2026-11-19T10:00:00Z"
    );
}

#[tokio::test]
async fn the_link_files_name_the_registered_apps() {
    let (db, _guard, _) = seeded().await;
    let android = android_file(&store::platforms_of_kind(&db, "android").await.unwrap());
    assert_eq!(android[0]["target"]["package_name"], "com.acme.app");
    assert_eq!(android[0]["target"]["sha256_cert_fingerprints"][1], "CC:DD");
    let apple = apple_file(&store::platforms_of_kind(&db, "ios").await.unwrap());
    let detail = &apple["applinks"]["details"][0];
    assert_eq!(detail["appIDs"][0], "TEAM123456.com.acme.app.staging");
    assert_eq!(detail["components"][0]["/"], "/events/*");
}

#[test]
fn the_admin_screens_load() {
    let module = rainmill_mobile::module();
    let mut registry = laterite_core::Registry::default();
    registry.set_owner(module.id());
    module.register(&mut registry);
}
