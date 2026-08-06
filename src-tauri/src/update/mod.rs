//! In-place updates.
//!
//! **The shape of the thing.** GitHub cannot push to an installed app — it is
//! on someone's laptop behind their router, with no address to call. So the
//! app asks: it fetches a small manifest from the latest release and compares
//! versions. Publishing a release is the trigger only in the sense that the
//! manifest at `releases/latest/download/latest.json` starts resolving to the
//! new file; every client picks it up on its next check.
//!
//! **Why the signature matters more than the download.** Whoever can answer
//! that HTTP request can hand the app a binary to run as the user. The
//! updater therefore verifies a minisign signature against a public key
//! compiled into the app, and refuses anything that does not match. An empty
//! `pubkey` is not "not configured yet" — it is the whole protection missing,
//! which is why this module treats a missing one as a hard failure rather
//! than degrading quietly.
//!
//! **What is testable here.** Talking to GitHub and swapping the bundle need
//! a real app handle and a real release, so they live in the commands. The
//! decision — *given what the updater found and what the user already said
//! no to, do we interrupt them?* — is pure, and that is what is tested.

use serde::Serialize;

/// The version the user asked not to be nagged about again.
pub const SETTING_SKIPPED_VERSION: &str = "update.skipped_version";

/// What to do about whatever the updater found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Nothing newer exists.
    UpToDate,
    /// Something newer exists, and the user already declined this exact one.
    Skipped,
    /// Something newer exists and the user has not seen it.
    Offer,
}

/// Everything the UI needs to describe the situation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// What is running now.
    pub current_version: String,
    /// What is available, when that is newer than what is running.
    pub available_version: Option<String>,
    /// Release notes, when the manifest carried any.
    pub notes: Option<String>,
    pub decision: Decision,
}

/// Whether to interrupt the user.
///
/// `available` is `None` when the updater found nothing newer — the plugin
/// does the version comparison, so this deliberately does not repeat it. What
/// it adds is the skip rule, and one case that is easy to get wrong: skipping
/// 0.2.0 must not silence 0.3.0. A skip that swallowed every later version
/// would strand someone on an old build permanently, and they would never see
/// a prompt again to tell them so.
pub fn decide(available: Option<&str>, skipped: Option<&str>) -> Decision {
    let Some(available) = available else {
        return Decision::UpToDate;
    };
    match skipped {
        // An empty setting is "nothing skipped", not "skipped the empty
        // version" — a stored empty string must never match a real version.
        Some(skipped) if !skipped.is_empty() && skipped == available => Decision::Skipped,
        _ => Decision::Offer,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_newer_means_nothing_to_say() {
        assert_eq!(decide(None, None), Decision::UpToDate);
        assert_eq!(decide(None, Some("9.9.9")), Decision::UpToDate);
    }

    #[test]
    fn a_new_version_is_offered() {
        assert_eq!(decide(Some("0.2.0"), None), Decision::Offer);
    }

    #[test]
    fn the_exact_version_the_user_declined_stays_quiet() {
        assert_eq!(decide(Some("0.2.0"), Some("0.2.0")), Decision::Skipped);
    }

    #[test]
    fn skipping_one_version_does_not_silence_the_next() {
        // The bug this exists to prevent: a skip that swallows every later
        // release strands the user on an old build with no prompt to tell
        // them so.
        assert_eq!(decide(Some("0.3.0"), Some("0.2.0")), Decision::Offer);
    }

    #[test]
    fn an_empty_skip_setting_is_not_a_skipped_version() {
        // `get_setting` returning Some("") must not match a real version, or
        // a blank row in the database silently disables updates.
        assert_eq!(decide(Some("0.2.0"), Some("")), Decision::Offer);
    }

    #[test]
    fn the_status_serialises_the_way_the_ui_reads_it() {
        // The frontend switches on these exact strings; renaming a variant
        // without renaming it there fails silently at runtime.
        let json = serde_json::to_string(&UpdateStatus {
            current_version: "0.1.0".into(),
            available_version: Some("0.2.0".into()),
            notes: None,
            decision: Decision::Offer,
        })
        .unwrap();
        assert!(json.contains(r#""currentVersion":"0.1.0""#));
        assert!(json.contains(r#""availableVersion":"0.2.0""#));
        assert!(json.contains(r#""decision":"offer""#));
    }

    #[test]
    fn every_decision_has_a_distinct_wire_name() {
        let name = |d: Decision| serde_json::to_string(&d).unwrap();
        assert_eq!(name(Decision::UpToDate), r#""up_to_date""#);
        assert_eq!(name(Decision::Skipped), r#""skipped""#);
        assert_eq!(name(Decision::Offer), r#""offer""#);
    }
}
