//! Preflight on a Google Play credential's page: each link of the chain tried
//! against Google on its own row, before any pipeline depends on it.

use laterite_core::{t, SealedFile, Text};
use rainmill_vault::action::{Action, ActionCx, Part, PartResult, State};

use crate::play::{Play, PlayError, ServiceAccount};

pub(crate) struct Preflight;

fn client(cx: &ActionCx<'_>) -> Result<Play, PlayError> {
    let key = cx
        .field("key")
        .and_then(SealedFile::parse)
        .ok_or_else(|| PlayError("no key file is stored".into()))?;
    Play::new(
        ServiceAccount::parse(&key.bytes)?,
        cx.field("package_name").unwrap_or_default(),
    )
}

#[laterite_core::strata::async_trait]
impl Action for Preflight {
    fn parts(&self) -> Vec<Part> {
        vec![
            Part::new("key", t!("Service account key")),
            Part::new("app", t!("Developer API enabled, and access to the app")),
        ]
    }

    fn note(&self) -> Option<Text> {
        Some(t!(
            "Each part tried against Google. A draft edit is opened and discarded; nothing in Play Console changes. The permission to release is only proven by an upload."
        ))
    }

    async fn part(&self, cx: &ActionCx<'_>, code: &str) -> PartResult {
        let client = match client(cx) {
            Ok(c) => c,
            Err(e) => return PartResult::new(State::Failed, e.0),
        };
        let token = match client.token().await {
            Ok(t) => t,
            Err(e) => return PartResult::new(State::Failed, e.0),
        };
        if code == "key" {
            return PartResult::new(
                State::Works,
                format!("Google accepted {}", client.account()),
            )
            .accepted();
        }
        match client.tracks(&token).await {
            Ok(tracks) => {
                let held: Vec<String> = tracks
                    .iter()
                    .map(|t| match t.build {
                        Some(build) => format!("{} holds build {build} ({})", t.name, t.status),
                        None => format!("{} is empty", t.name),
                    })
                    .collect();
                PartResult::new(
                    State::Works,
                    if held.is_empty() {
                        "The app is visible; no track has a release yet. Upload the first build by hand in Play Console.".to_string()
                    } else {
                        format!("The app is visible: {}.", held.join(", "))
                    },
                )
            }
            Err(e) => PartResult::new(State::Failed, e.0),
        }
    }
}
