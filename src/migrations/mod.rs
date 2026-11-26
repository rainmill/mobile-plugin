//! This plugin's migrations, in apply order.

laterite_core::migration_set! {
    module_id: "rainmill.mobile",
    m0001_create_mobile,
    m0002_add_build_files,
    m0003_add_policy_dimensions,
}
