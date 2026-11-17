//! The publishing guide: the steps from no store listing to a release that
//! publishes itself, in order, each saying where it is done and what it needs.

use laterite_admin::axum::routing::get;
use laterite_admin::axum::{Extension, Router};
use laterite_admin::html::Markup;
use laterite_admin::routes::{RouteCtx, Screen};
use laterite_admin::{PageHead, Shell};
use laterite_core::t;

/// Where a step is done.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Place {
    /// Google Play Console.
    PlayConsole,
    /// Google Cloud Console.
    CloudConsole,
    /// This admin.
    Here,
    /// The app's repository and its pipeline.
    Repository,
}

impl Place {
    fn label(self) -> &'static str {
        match self {
            Place::PlayConsole => "Play Console",
            Place::CloudConsole => "Google Cloud Console",
            Place::Here => "Here",
            Place::Repository => "Your repository",
        }
    }
}

/// One step of the guide.
pub struct Step {
    /// Where it is done.
    pub place: Place,
    /// What is done, in a few words.
    pub title: &'static str,
    /// How, and what goes wrong.
    pub detail: &'static str,
    /// Whether only a person may do it: a sign-in, a declaration, a key.
    pub yours: bool,
}

const fn step(place: Place, title: &'static str, detail: &'static str, yours: bool) -> Step {
    Step {
        place,
        title,
        detail,
        yours,
    }
}

/// Publishing an Android app to Google Play, in order.
pub const ANDROID: &[Step] = &[
    step(Place::PlayConsole, "Open a developer account and verify it",
        "An organisation account needs a D-U-N-S number and business verification. A new personal account must run a closed test with testers before it may publish to production. Allow days for verification.", true),
    step(Place::PlayConsole, "Create the app",
        "Name, default language, app or game, free or paid. The package name is fixed by the first build you upload and can never change.", true),
    step(Place::PlayConsole, "Make the declarations",
        "Store listing, content rating, data safety, target audience, ads, app access, and a privacy policy address. These are statements you answer for, some of them in law, so no tool fills them in. Including children in the audience changes what the privacy policy, ads and analytics may do.", true),
    step(Place::PlayConsole, "Declare trader status",
        "Declared for the account, then confirmed for each app. It turns on whether you act for a trade or business, not on whether the app earns money. A trader needs a terms of service address for each app, so write and host that page first.", true),
    step(Place::Repository, "Create an upload key and enrol in Play App Signing",
        "Google keeps the app signing key; you keep the upload key, which can be reset from Play Console if it is lost. Keep the keystore outside the repository and its passwords out of version control.", true),
    step(Place::PlayConsole, "Upload the first build by hand",
        "Upload a signed bundle to a testing track or to production and send it for review. Review took about four days in our experience. Before promoting a track, check which build it holds: promoting ships that one.", true),
    step(Place::CloudConsole, "Link a Google Cloud project and enable the Developer API",
        "APIs and Services, Library, Google Play Android Developer API, Enable. Creating a service account does not enable it, and nothing says so until the last step of a pipeline.", false),
    step(Place::CloudConsole, "Create a service account and download its key",
        "IAM and Admin, Service accounts, Create. Give it no Cloud roles. Under Keys, add a JSON key: that file is the credential.", true),
    step(Place::PlayConsole, "Invite the service account",
        "Users and permissions, Invite new users, paste the service account's address, add this one app, and grant only \"View app information\" and \"Release to production, exclude devices and use Play App Signing\". Never Admin. Guides that send you to \"API access\" describe a page that no longer exists.", true),
    step(Place::Here, "Store the key and run the preflight",
        "Settings, Credentials, New credential, Google Play. Choose the app it belongs to, add the key file and the package name, then press Preflight. Fix what it reports before writing a pipeline: each of these otherwise fails one at a time, at the end of a run.", false),
    step(Place::Repository, "Publish from a version tag",
        "Publish only when a version tag is pushed, never on an ordinary push. Derive the version code from the version (major x 10000 + minor x 100 + patch) so the two cannot drift. Build a downloadable artifact first, so a refusal from Play still leaves a way to install. Upload as a draft and start the rollout yourself.", false),
    step(Place::Repository, "Make the pipeline prove it uploaded",
        "A pipeline that skips the upload when its secrets are missing still reports success. Print one unmistakable line after the upload and fail the run without it. Test on a throwaway tag first: a tag-triggered run builds the tag, so a fix after tagging means moving the tag.", false),
    step(Place::Repository, "Write each release note",
        "At most 500 characters, in a folder named for the store listing's language. A longer note should fail the release, not be cut short.", false),
];

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn render(shell: &Shell, steps: &[Step], credentials: &str) -> String {
    let mut body = format!(
        "<p class=\"lat-note\">{}</p><div class=\"lat-card\"><ol class=\"lat-prose\" style=\"margin:0;padding-left:1.4em\">",
        escape(&shell.t("Android, from no store listing to a release that publishes itself. Steps marked \"You\" need a person: a sign-in, a declaration or a key."))
    );
    for s in steps {
        body.push_str(&format!(
            "<li style=\"margin-bottom:14px\"><strong>{}</strong> <span class=\"lat-muted\">({}{})</span><br>{}</li>",
            escape(&shell.t(s.title)),
            escape(&shell.t(s.place.label())),
            if s.yours { format!(", {}", escape(&shell.t("You"))) } else { String::new() },
            escape(&shell.t(s.detail)),
        ));
    }
    body.push_str(&format!(
        "</ol></div><p style=\"margin-top:16px\"><a class=\"lat-btn lat-btn--primary\" href=\"{}\">{}</a></p><p class=\"lat-note\">{}</p>",
        escape(credentials),
        escape(&shell.t("Open Credentials")),
        escape(&shell.t("The App Store guide is not written yet.")),
    ));
    body
}

pub(crate) struct Guide;

impl Screen for Guide {
    fn mount(&self, ctx: &RouteCtx) -> Router {
        let credentials = format!("{}/rainmill/vault/credentials", ctx.admin_path());
        Router::new().route(
            "/",
            get(move |Extension(shell): Extension<Shell>| {
                let credentials = credentials.clone();
                async move {
                    let body = render(&shell, ANDROID, &credentials);
                    shell.page_with(
                        PageHead::new(t!("Publishing guide")),
                        Markup::raw_unchecked(body),
                    )
                }
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_api_is_enabled_before_the_key_is_stored_and_tested() {
        let at = |needle: &str| {
            ANDROID
                .iter()
                .position(|s| s.title.contains(needle))
                .unwrap()
        };
        assert!(at("enable the Developer API") < at("run the preflight"));
        assert!(at("Invite the service account") < at("run the preflight"));
        assert!(at("run the preflight") < at("Publish from a version tag"));
        assert!(ANDROID
            .iter()
            .all(|s| s.detail.len() < 420 && !s.detail.contains(" — ")));
    }
}
