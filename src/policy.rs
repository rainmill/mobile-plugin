//! The policy engine: which flags, maintenance notice, update demand and
//! banners apply to one install, decided in one place.

use std::collections::BTreeMap;

use laterite_core::serde_json::{self, Value};
use serde::{Deserialize, Serialize};

/// What a policy is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A named value the app reads.
    Flag,
    /// The app shows the message and stops.
    Maintenance,
    /// The app asks for, or insists on, a newer build.
    Update,
    /// A message shown once on launch.
    Banner,
}

impl Kind {
    /// The kind a stored code names.
    pub fn parse(code: &str) -> Option<Self> {
        match code {
            "flag" => Some(Self::Flag),
            "maintenance" => Some(Self::Maintenance),
            "update" => Some(Self::Update),
            "banner" => Some(Self::Banner),
            _ => None,
        }
    }
}

/// A policy as stored: what it is, and who it applies to. An empty condition
/// applies to every install.
#[derive(Clone, Debug)]
pub struct Policy {
    /// The row's id; among flags of one key, the highest id that applies wins.
    pub id: i64,
    /// What it is.
    pub kind: Kind,
    /// A flag's or banner's name.
    pub key: String,
    /// A flag's value, or `soft` or `hard` for an update.
    pub value: String,
    /// What the person reads.
    pub message: String,
    /// Only this platform (`android`, `ios`, `web`).
    pub platform: Option<String>,
    /// Only this environment.
    pub environment: Option<String>,
    /// Only builds from this one.
    pub min_build: Option<i64>,
    /// Only builds up to this one.
    pub max_build: Option<i64>,
    /// Only these locales; `en` matches `en-IN`.
    pub locales: Vec<String>,
    /// Only this share of installs, 1 to 100.
    pub percent: Option<i64>,
    /// From this moment (UTC, RFC 3339).
    pub starts_at: Option<String>,
    /// Until this moment.
    pub ends_at: Option<String>,
}

/// The install a verdict is for.
#[derive(Clone, Debug)]
pub struct Subject {
    /// `android`, `ios` or `web`.
    pub platform: String,
    /// The environment of the variant it was built as.
    pub environment: String,
    /// Its build number.
    pub build: i64,
    /// Its locale (`en-IN`), empty when unknown.
    pub locale: String,
    /// Its install id, which places it in a percentage.
    pub install_id: String,
}

/// The platform's own update floors and newest release.
#[derive(Clone, Debug, Default)]
pub struct Floors {
    /// Builds below this must update.
    pub min_build: Option<i64>,
    /// Builds below this are asked to.
    pub recommended_build: Option<i64>,
    /// The newest released build and its version.
    pub latest: Option<(i64, String)>,
    /// Where the app is updated from.
    pub store_url: Option<String>,
}

/// How strongly an update is asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// The build is current enough.
    None,
    /// A prompt the person may dismiss.
    Soft,
    /// The app stops until it is updated.
    Hard,
}

/// The update part of a verdict.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Update {
    /// How strongly it is asked for.
    pub level: Level,
    /// What the person reads; the library's own words when empty.
    pub message: String,
    /// Where the app is updated from.
    pub store_url: Option<String>,
    /// The newest released build.
    pub latest_build: Option<i64>,
    /// The newest released version.
    pub latest_version: Option<String>,
}

/// The maintenance part of a verdict, present while the app is closed.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Maintenance {
    /// What the person reads.
    pub message: String,
    /// When maintenance ends, if it was given an end.
    pub until: Option<String>,
}

/// A banner to show.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Banner {
    /// Its name; the app shows each once.
    pub key: String,
    /// What the person reads.
    pub message: String,
}

/// What an install is told.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// Whether, and how strongly, to update.
    pub update: Update,
    /// Set while the app is closed for maintenance.
    pub maintenance: Option<Maintenance>,
    /// Flags by name.
    pub flags: BTreeMap<String, Value>,
    /// Banners to show.
    pub banners: Vec<Banner>,
}

/// A stable number from 0 to 99 for an install and a policy, so a percentage
/// keeps the same installs as it grows.
fn bucket(install_id: &str, key: &str) -> i64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in install_id.bytes().chain(*b":").chain(key.bytes()) {
        hash = (hash ^ u64::from(b)).wrapping_mul(0x100000001b3);
    }
    (hash % 100) as i64
}

/// A flag's value as the app reads it: `true`, `false`, a number, JSON, or
/// the text itself.
pub fn flag_value(raw: &str) -> Value {
    let raw = raw.trim();
    if raw.is_empty() {
        return Value::Bool(true);
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(v) if !v.is_null() => v,
        _ => Value::String(raw.to_string()),
    }
}

impl Policy {
    /// Whether the policy applies to `subject` at `now` (UTC, RFC 3339).
    pub fn applies(&self, subject: &Subject, now: &str) -> bool {
        let is = |want: &Option<String>, have: &str| {
            want.as_deref().is_none_or(|w| w.is_empty() || w == have)
        };
        let locale = subject.locale.to_ascii_lowercase();
        is(&self.platform, &subject.platform)
            && is(&self.environment, &subject.environment)
            && self.min_build.is_none_or(|m| subject.build >= m)
            && self.max_build.is_none_or(|m| subject.build <= m)
            && (self.locales.is_empty()
                || self.locales.iter().any(|l| {
                    let l = l.to_ascii_lowercase();
                    locale == l || locale.starts_with(&format!("{l}-"))
                }))
            && self
                .percent
                .is_none_or(|p| bucket(&subject.install_id, &self.key) < p.clamp(0, 100))
            && self
                .starts_at
                .as_deref()
                .is_none_or(|s| s.is_empty() || s <= now)
            && self
                .ends_at
                .as_deref()
                .is_none_or(|e| e.is_empty() || now < e)
    }
}

/// The verdict for `subject`: the platform's floors, then every policy that
/// applies. The strongest update demand wins; among flags of one name, the
/// newest policy that applies.
pub fn evaluate(policies: &[Policy], floors: &Floors, subject: &Subject, now: &str) -> Verdict {
    let mut level = Level::None;
    if floors.recommended_build.is_some_and(|b| subject.build < b) {
        level = Level::Soft;
    }
    if floors.min_build.is_some_and(|b| subject.build < b) {
        level = Level::Hard;
    }
    let mut verdict = Verdict {
        update: Update {
            level,
            message: String::new(),
            store_url: floors.store_url.clone(),
            latest_build: floors.latest.as_ref().map(|l| l.0),
            latest_version: floors.latest.as_ref().map(|l| l.1.clone()),
        },
        maintenance: None,
        flags: BTreeMap::new(),
        banners: Vec::new(),
    };
    let mut applying: Vec<&Policy> = policies
        .iter()
        .filter(|p| p.applies(subject, now))
        .collect();
    applying.sort_by_key(|p| p.id);
    for policy in applying {
        match policy.kind {
            Kind::Flag => {
                verdict
                    .flags
                    .insert(policy.key.clone(), flag_value(&policy.value));
            }
            Kind::Maintenance => {
                verdict.maintenance = Some(Maintenance {
                    message: policy.message.clone(),
                    until: policy.ends_at.clone().filter(|e| !e.is_empty()),
                });
            }
            Kind::Update => {
                let asked = if policy.value.trim() == "hard" {
                    Level::Hard
                } else {
                    Level::Soft
                };
                if asked >= verdict.update.level {
                    verdict.update.level = asked;
                    if !policy.message.is_empty() {
                        verdict.update.message = policy.message.clone();
                    }
                }
            }
            Kind::Banner => verdict.banners.push(Banner {
                key: policy.key.clone(),
                message: policy.message.clone(),
            }),
        }
    }
    verdict
}

#[cfg(test)]
mod tests {
    use super::*;
    use laterite_core::serde_json::json;

    fn policy(id: i64, kind: Kind, key: &str, value: &str) -> Policy {
        Policy {
            id,
            kind,
            key: key.into(),
            value: value.into(),
            message: String::new(),
            platform: None,
            environment: None,
            min_build: None,
            max_build: None,
            locales: Vec::new(),
            percent: None,
            starts_at: None,
            ends_at: None,
        }
    }

    fn subject(build: i64) -> Subject {
        Subject {
            platform: "android".into(),
            environment: "production".into(),
            build,
            locale: "en-IN".into(),
            install_id: "install-1".into(),
        }
    }

    const NOW: &str = "2026-11-18T10:00:00Z";

    #[test]
    fn floors_ask_for_or_insist_on_an_update() {
        let floors = Floors {
            min_build: Some(10),
            recommended_build: Some(20),
            latest: Some((25, "1.5.0".into())),
            store_url: Some("https://acme.com/app".into()),
        };
        assert_eq!(
            evaluate(&[], &floors, &subject(9), NOW).update.level,
            Level::Hard
        );
        assert_eq!(
            evaluate(&[], &floors, &subject(15), NOW).update.level,
            Level::Soft
        );
        let current = evaluate(&[], &floors, &subject(20), NOW);
        assert_eq!(current.update.level, Level::None);
        assert_eq!(current.update.latest_version.as_deref(), Some("1.5.0"));
    }

    #[test]
    fn an_update_policy_raises_the_demand_for_the_builds_it_names() {
        let mut p = policy(1, Kind::Update, "bad-builds", "hard");
        p.min_build = Some(12);
        p.max_build = Some(14);
        p.message = "This build loses drafts. Please update.".into();
        let floors = Floors::default();
        let hit = evaluate(std::slice::from_ref(&p), &floors, &subject(13), NOW);
        assert_eq!(
            (hit.update.level, hit.update.message.as_str()),
            (Level::Hard, "This build loses drafts. Please update.")
        );
        assert_eq!(
            evaluate(&[p], &floors, &subject(15), NOW).update.level,
            Level::None
        );
    }

    #[test]
    fn conditions_narrow_who_a_policy_applies_to() {
        let s = subject(5);
        let mut p = policy(1, Kind::Flag, "new_feed", "true");
        assert!(p.applies(&s, NOW));
        p.platform = Some("ios".into());
        assert!(!p.applies(&s, NOW));
        p.platform = Some("android".into());
        p.environment = Some("staging".into());
        assert!(!p.applies(&s, NOW));
        p.environment = None;
        p.locales = vec!["en".into()];
        assert!(p.applies(&s, NOW));
        p.locales = vec!["hi".into()];
        assert!(!p.applies(&s, NOW));
        p.locales.clear();
        p.starts_at = Some("2026-11-19T00:00:00Z".into());
        assert!(!p.applies(&s, NOW));
        p.starts_at = None;
        p.ends_at = Some("2026-11-18T10:00:00Z".into());
        assert!(!p.applies(&s, NOW), "the end is not included");
    }

    #[test]
    fn a_percentage_keeps_its_installs_as_it_grows() {
        let mut p = policy(1, Kind::Flag, "new_feed", "true");
        let installs: Vec<Subject> = (0..400)
            .map(|i| Subject {
                install_id: format!("install-{i}"),
                ..subject(1)
            })
            .collect();
        p.percent = Some(20);
        let at_20: Vec<bool> = installs.iter().map(|s| p.applies(s, NOW)).collect();
        let share = at_20.iter().filter(|b| **b).count();
        assert!(
            (50..=110).contains(&share),
            "about a fifth of 400, got {share}"
        );
        p.percent = Some(60);
        for (s, was) in installs.iter().zip(&at_20) {
            assert!(
                !was || p.applies(s, NOW),
                "an install in 20% stays in at 60%"
            );
        }
        p.percent = Some(100);
        assert!(installs.iter().all(|s| p.applies(s, NOW)));
    }

    #[test]
    fn flags_read_as_their_type_and_the_newest_wins() {
        assert_eq!(flag_value("true"), json!(true));
        assert_eq!(flag_value(""), json!(true));
        assert_eq!(flag_value("3"), json!(3));
        assert_eq!(flag_value("blue"), json!("blue"));
        assert_eq!(flag_value(r#"{"max": 5}"#), json!({ "max": 5 }));
        let mut narrow = policy(2, Kind::Flag, "theme", "dark");
        narrow.environment = Some("staging".into());
        let all = [policy(1, Kind::Flag, "theme", "light"), narrow];
        assert_eq!(
            evaluate(&all, &Floors::default(), &subject(1), NOW).flags["theme"],
            json!("light")
        );
        let staging = Subject {
            environment: "staging".into(),
            ..subject(1)
        };
        assert_eq!(
            evaluate(&all, &Floors::default(), &staging, NOW).flags["theme"],
            json!("dark")
        );
    }

    #[test]
    fn maintenance_and_banners_carry_their_message() {
        let mut m = policy(1, Kind::Maintenance, "upgrade", "");
        m.message = "Back at 6 pm.".into();
        m.ends_at = Some("2026-11-18T12:30:00Z".into());
        let mut b = policy(2, Kind::Banner, "diwali", "");
        b.message = "Happy Diwali!".into();
        let v = evaluate(&[m, b], &Floors::default(), &subject(1), NOW);
        assert_eq!(
            v.maintenance,
            Some(Maintenance {
                message: "Back at 6 pm.".into(),
                until: Some("2026-11-18T12:30:00Z".into())
            })
        );
        assert_eq!(
            v.banners,
            vec![Banner {
                key: "diwali".into(),
                message: "Happy Diwali!".into()
            }]
        );
    }
}
