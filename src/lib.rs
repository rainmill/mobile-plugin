//! App management for a Laterite application: apps with their Android, iOS
//! and web platforms, one policy engine for feature flags, maintenance and
//! required updates, release records with changelogs, and the devices that
//! check in.
//!
//! An app checks in at [`api::HELLO_PATH`] when it starts and is told what
//! applies to it; see [`policy::Verdict`].
#![warn(missing_docs)]

pub mod api;
mod migrations;
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
