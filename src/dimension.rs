//! Target dimensions: a condition of the application's own that a policy
//! waits on.
//!
//! The engine knows platforms, builds, locales and shares of installs. What
//! else a policy may depend on is the application's to say: the place a
//! request is about, the plan an account is on. The application registers a
//! [`TargetDimension`] under a name, a policy names it, and an app sends the
//! dimension's value when it checks in (`"context": { "location": "42" }`).
//!
//! A policy that is switched off is never put to its dimensions: the switch
//! decides first, and only a policy that is on asks them.
//!
//! ```ignore
//! struct Location;
//!
//! #[async_trait]
//! impl TargetDimension for Location {
//!     fn name(&self) -> &'static str { "location" }
//!     async fn allows(&self, db: &Db, policy: &str, value: Option<&str>) -> bool {
//!         place_allows(db, policy, value).await
//!     }
//! }
//!
//! registry.add_target_dimension(Arc::new(Location));
//! ```

use std::collections::BTreeMap;
use std::sync::Arc;

use laterite_core::strata::async_trait;
use laterite_core::{Db, Registry};

use crate::policy::Policy;

/// A condition of the application's own that a policy can wait on.
#[async_trait]
pub trait TargetDimension: Send + Sync + 'static {
    /// The name a policy lists and an app sends its value under.
    fn name(&self) -> &'static str;

    /// Whether the policy keyed `policy` applies for `value`: what the app
    /// sent for this dimension, or `None` when it sent nothing.
    async fn allows(&self, db: &Db, policy: &str, value: Option<&str>) -> bool;
}

/// A registered dimension.
pub struct TargetDimensionReg(pub Arc<dyn TargetDimension>);

/// Registers target dimensions.
pub trait MobileRegistry {
    /// Adds a dimension a policy can name.
    fn add_target_dimension(&mut self, dimension: Arc<dyn TargetDimension>);
}

impl MobileRegistry for Registry {
    fn add_target_dimension(&mut self, dimension: Arc<dyn TargetDimension>) {
        self.add(TargetDimensionReg(dimension));
    }
}

/// The policies among `policies` whose every dimension allows them for the
/// values in `context`. A policy naming a dimension nobody registered does
/// not apply: an unknown condition is one that cannot be met.
pub async fn allowed(
    db: &Db,
    dimensions: &[Arc<dyn TargetDimension>],
    policies: Vec<Policy>,
    context: &BTreeMap<String, String>,
) -> Vec<Policy> {
    let mut out = Vec::with_capacity(policies.len());
    'policies: for policy in policies {
        for name in &policy.dimensions {
            let Some(dimension) = dimensions.iter().find(|d| d.name() == name) else {
                continue 'policies;
            };
            let value = context.get(name).map(String::as_str);
            if !dimension.allows(db, &policy.key, value).await {
                continue 'policies;
            }
        }
        out.push(policy);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::policy::Kind;
    use laterite_core::testing::connect_test;

    /// Allows a policy where the app says the place is `open`.
    struct Place;

    #[async_trait]
    impl TargetDimension for Place {
        fn name(&self) -> &'static str {
            "place"
        }
        async fn allows(&self, _db: &Db, policy: &str, value: Option<&str>) -> bool {
            policy == "guestlist" && value == Some("open")
        }
    }

    fn flag(id: i64, key: &str, dimensions: &[&str]) -> Policy {
        Policy {
            id,
            kind: Kind::Flag,
            key: key.to_string(),
            value: "true".to_string(),
            message: String::new(),
            platform: None,
            environment: None,
            min_build: None,
            max_build: None,
            locales: Vec::new(),
            percent: None,
            starts_at: None,
            ends_at: None,
            dimensions: dimensions.iter().map(|d| d.to_string()).collect(),
        }
    }

    #[tokio::test]
    async fn a_policy_applies_where_each_of_its_dimensions_allows_it() {
        let (db, _guard) = connect_test(&[crate::migrations::migrations()]).await;
        let dimensions: Vec<Arc<dyn TargetDimension>> = vec![Arc::new(Place)];
        let policies = || {
            vec![
                flag(1, "feed", &[]),
                flag(2, "guestlist", &["place"]),
                flag(3, "offers", &["plan"]),
            ]
        };
        let keys = |policies: Vec<Policy>| -> Vec<String> {
            policies.into_iter().map(|p| p.key).collect()
        };

        // The place allows it: both the plain flag and the one that waited.
        let open = BTreeMap::from([("place".to_string(), "open".to_string())]);
        assert_eq!(
            keys(allowed(&db, &dimensions, policies(), &open).await),
            ["feed", "guestlist"]
        );
        // Another place, or none sent: only the flag with no conditions. A
        // dimension nobody registered never allows.
        let shut = BTreeMap::from([("place".to_string(), "shut".to_string())]);
        assert_eq!(
            keys(allowed(&db, &dimensions, policies(), &shut).await),
            ["feed"]
        );
        assert_eq!(
            keys(allowed(&db, &dimensions, policies(), &BTreeMap::new()).await),
            ["feed"]
        );
    }
}
