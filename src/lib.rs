//! App management for a Laterite application: apps with their Android, iOS
//! and web platforms, one policy engine for feature flags, maintenance and
//! required updates, release records with changelogs, and the devices that
//! check in.
//!
//! An app checks in at [`api::HELLO_PATH`] when it starts and is told what
//! applies to it; see [`policy::Verdict`].
#![warn(missing_docs)]

pub mod api;
mod check;
pub mod guide;
mod migrations;
pub mod play;
pub mod policy;
pub mod store;

use std::sync::Arc;

use laterite_admin::routes::PublicRouteReg;
use laterite_admin::{AdminRegistry, Permission};
use laterite_core::strata::async_trait;
use laterite_core::t;
use laterite_core::{MigrationSet, Module, ModuleId, Registry};

/// The module's id.
pub const MODULE_ID: &str = migrations::MODULE_ID;
/// Managing apps, their platforms, policies and releases.
pub const MANAGE: &str = "rainmill.mobile.manage";
/// Seeing the devices that checked in.
pub const DEVICES: &str = "rainmill.mobile.devices";
/// The vault scope a credential of one app carries.
pub const APP_SCOPE: &str = "rainmill.mobile.app";
/// The vault kind of a Google Play service account.
pub const PLAY_KIND: &str = "rainmill.mobile.google_play";
/// The vault slot a release to Google Play uses.
pub const PLAY_SLOT: &str = "rainmill.mobile.google_play";
/// The menu section holding the screens.
pub const SECTION: &str = "rainmill.mobile";

/// This plugin's entry point, collected by the generated plugins manifest.
pub fn module() -> Box<dyn Module> {
    Box::new(MobileModule)
}

/// The Rainmill Mobile plugin.
pub struct MobileModule;

const REQUIRES: &[ModuleId] = &[ModuleId::new("rainmill.vault")];

#[async_trait]
impl Module for MobileModule {
    fn id(&self) -> ModuleId {
        ModuleId::new(MODULE_ID)
    }

    fn migrations(&self) -> MigrationSet {
        migrations::migrations()
    }

    fn requires(&self) -> &'static [ModuleId] {
        REQUIRES
    }

    fn register(&self, registry: &mut Registry) {
        registry.add(rainmill_vault::ScopeReg::table(
            APP_SCOPE,
            t!("App"),
            "mobile_apps",
            "name",
        ));
        let group = || t!("Apps");
        registry.add_permission(Permission::new(
            MANAGE,
            t!("Manage apps, flags, maintenance and releases"),
            group(),
        ));
        registry.add_permission(Permission::new(
            DEVICES,
            t!("See the devices that checked in"),
            group(),
        ));
        registry
            .add_nav(laterite_admin::nav::NavItem::section(SECTION, t!("Apps")).icon("smartphone"));
        registry.add_resource(laterite_admin::resource!("admin/apps.yaml"));
        registry.add_resource(laterite_admin::resource!("admin/devices.yaml"));
        registry.add_screen(
            laterite_admin::routes::ScreenReg::new("/guide", MANAGE, Arc::new(guide::Guide))
                .in_menu(t!("Publishing guide"))
                .menu(
                    laterite_admin::nav::Placement::new()
                        .under(SECTION)
                        .icon("rocket")
                        .order(3),
                ),
        );
        registry.add(
            rainmill_vault::KindReg::new(PLAY_KIND, t!("Google Play"))
                .help(t!(
                    "A Google Cloud service account invited to the app in Play Console, with the Google Play Android Developer API enabled on its project. The publishing guide under Apps has the steps."
                ))
                .field(
                    laterite_admin::form::FormField::of("key", t!("Service account key"), "sensitive")
                        .options(laterite_core::serde_json::json!({ "file": true, "accept": [".json"], "max_size": "16kb" }))
                        .required()
                        .help(t!("The JSON key file Google Cloud downloads under the service account's Keys.")),
                )
                .field(
                    laterite_admin::form::FormField::text("package_name", t!("Package name"))
                        .required()
                        .help(t!("Such as com.acme.app, as Play Console shows it.")),
                ),
        );
        registry.add(
            rainmill_vault::SlotReg::single(PLAY_SLOT, PLAY_KIND)
                .label(t!("Releases to Google Play"))
                .reason(t!(
                    "Reads an app's tracks, and uploads a release when asked to."
                )),
        );
        registry.add(
            rainmill_vault::ActionReg::new(
                PLAY_KIND,
                "preflight",
                t!("Preflight"),
                Arc::new(check::Preflight),
            )
            .icon("plug"),
        );
        // An app authenticates nothing here and sends no Origin; the endpoint
        // refuses any identifier that is not registered.
        registry.add_public_route(
            PublicRouteReg::new(api::HELLO_PATH, api::hello_route()).skip_origin_check(),
        );
        registry.add_public_route(PublicRouteReg::new(
            api::APPLE_PATH,
            Arc::new(api::LinkFile("ios")),
        ));
        registry.add_public_route(PublicRouteReg::new(
            api::ANDROID_PATH,
            Arc::new(api::LinkFile("android")),
        ));
    }
}
