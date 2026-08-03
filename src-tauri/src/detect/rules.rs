//! Which apps may offer to record.
//!
//! The single most important property here is that **nothing fires without an
//! explicit rule**. A popup every time something touches the microphone would
//! be worse than no detection at all — dictation tools, voice memos, and a
//! browser tab playing a video all hold the input device, and the user named
//! this problem directly when they asked for per-app control.
//!
//! So the closed state is the default. An app is offered only when a rule says
//! so; an app the user has refused never asks again; and an app nobody has
//! ruled on gets exactly one question.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// A stored per-app rule.
///
/// Mirrors `detection_rules.mode`, which is constrained to these two values —
/// "ask again later" is deliberately not representable, because a question the
/// user has already answered should not come back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleMode {
    Allow,
    Ignore,
}

impl RuleMode {
    pub fn as_str(self) -> &'static str {
        match self {
            RuleMode::Allow => "allow",
            RuleMode::Ignore => "ignore",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "allow" => Some(RuleMode::Allow),
            "ignore" => Some(RuleMode::Ignore),
            _ => None,
        }
    }
}

/// What to do when an app starts using the microphone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// Put a candidate in the queue. Still only ever *offers* — consent to
    /// record is a separate step (G22).
    Offer,
    /// Do nothing at all, silently.
    Ignore,
    /// Ask the user once whether this app should count, then remember.
    Ask,
}

/// Apps that are meetings often enough to be worth offering out of the box.
///
/// A default, not a hard-coded truth: a stored rule always wins, so a user who
/// never wants Slack calls offered can say so and be obeyed.
///
/// Browsers are included because a great many calls happen in a tab, and this
/// is the only way to catch Meet or Webex-in-browser at all. The cost of being
/// wrong is one dismissed popup — the orchestrator never records without a
/// click — which is the right side of the trade for a feature whose failure
/// mode is "did not notice my meeting".
pub const BUILTIN_ALLOW: &[(&str, &str)] = &[
    ("us.zoom.xos", "Zoom"),
    ("com.microsoft.teams2", "Microsoft Teams"),
    ("com.microsoft.teams", "Microsoft Teams (classic)"),
    ("com.tinyspeck.slackmacgap", "Slack"),
    ("com.hnc.Discord", "Discord"),
    ("com.apple.FaceTime", "FaceTime"),
    ("Cisco-Systems.Spark", "Webex"),
    ("com.cisco.webexmeetingsapp", "Webex Meetings"),
    ("com.google.Chrome", "Google Chrome"),
    ("com.apple.Safari", "Safari"),
    ("company.thebrowser.Browser", "Arc"),
    ("com.brave.Browser", "Brave"),
    ("com.microsoft.edgemac", "Microsoft Edge"),
];

pub fn is_builtin_allowed(bundle_id: &str) -> bool {
    BUILTIN_ALLOW.iter().any(|(id, _)| *id == bundle_id)
}

pub fn builtin_name(bundle_id: &str) -> Option<&'static str> {
    BUILTIN_ALLOW
        .iter()
        .find(|(id, _)| *id == bundle_id)
        .map(|(_, name)| *name)
}

/// Decides what a microphone activation means.
///
/// `stored` is the user's own rules, which take precedence over the built-in
/// list in both directions — turning a shipped default off, or turning an app
/// we have never heard of on.
pub fn decide(bundle_id: &str, stored: &HashMap<String, RuleMode>) -> Decision {
    // An app with no bundle identifier cannot be ruled about at all: a rule has
    // to survive the process exiting, and a pid does not. The sidecar filters
    // these out already; this is the second gate, because "we never ask about
    // something we cannot remember the answer to" is a property worth holding
    // in the layer that makes the decision.
    if bundle_id.trim().is_empty() {
        return Decision::Ignore;
    }

    match stored.get(bundle_id) {
        Some(RuleMode::Allow) => Decision::Offer,
        Some(RuleMode::Ignore) => Decision::Ignore,
        None if is_builtin_allowed(bundle_id) => Decision::Offer,
        None => Decision::Ask,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stored(entries: &[(&str, RuleMode)]) -> HashMap<String, RuleMode> {
        entries
            .iter()
            .map(|(id, mode)| ((*id).to_string(), *mode))
            .collect()
    }

    #[test]
    fn a_shipped_meeting_app_is_offered_with_no_setup() {
        assert_eq!(decide("us.zoom.xos", &stored(&[])), Decision::Offer);
    }

    #[test]
    fn an_app_nobody_has_ruled_on_is_asked_about_once() {
        // The dictation case the user raised: it should ask, not act.
        assert_eq!(decide("com.wisprflow.app", &stored(&[])), Decision::Ask);
    }

    #[test]
    fn a_refusal_is_permanent() {
        // "Never" has to mean never. Asking again is the behaviour that makes
        // people turn a feature off entirely.
        assert_eq!(
            decide(
                "com.wisprflow.app",
                &stored(&[("com.wisprflow.app", RuleMode::Ignore)])
            ),
            Decision::Ignore
        );
    }

    #[test]
    fn a_user_rule_overrides_a_shipped_default() {
        // Someone who takes Slack huddles they never want recorded must be able
        // to say so and be obeyed.
        assert_eq!(
            decide(
                "com.tinyspeck.slackmacgap",
                &stored(&[("com.tinyspeck.slackmacgap", RuleMode::Ignore)])
            ),
            Decision::Ignore
        );
    }

    #[test]
    fn a_user_can_allow_an_app_we_have_never_heard_of() {
        assert_eq!(
            decide(
                "com.example.NewCallApp",
                &stored(&[("com.example.NewCallApp", RuleMode::Allow)])
            ),
            Decision::Offer
        );
    }

    #[test]
    fn an_app_with_no_identifier_is_never_asked_about() {
        // There would be nowhere to store the answer.
        assert_eq!(decide("", &stored(&[])), Decision::Ignore);
        assert_eq!(decide("   ", &stored(&[])), Decision::Ignore);
    }

    #[test]
    fn rules_for_other_apps_do_not_leak() {
        let rules = stored(&[("us.zoom.xos", RuleMode::Ignore)]);
        assert_eq!(decide("com.hnc.Discord", &rules), Decision::Offer);
        assert_eq!(decide("com.unknown.thing", &rules), Decision::Ask);
    }

    #[test]
    fn the_builtin_list_covers_what_the_spec_named() {
        // The roadmap names these explicitly; losing one is a silent regression
        // in what gets noticed.
        for id in [
            "us.zoom.xos",
            "com.microsoft.teams2",
            "com.tinyspeck.slackmacgap",
            "com.hnc.Discord",
            "com.apple.FaceTime",
            "com.google.Chrome",
            "com.apple.Safari",
            "company.thebrowser.Browser",
        ] {
            assert!(is_builtin_allowed(id), "{id} dropped from the allowlist");
        }
    }

    #[test]
    fn the_builtin_list_has_no_duplicate_ids() {
        let ids: std::collections::HashSet<_> = BUILTIN_ALLOW.iter().map(|(id, _)| id).collect();
        assert_eq!(ids.len(), BUILTIN_ALLOW.len());
    }

    #[test]
    fn every_builtin_has_a_display_name() {
        // The name is what the "always / never" question shows; an empty one
        // would ask about a blank.
        for (id, name) in BUILTIN_ALLOW {
            assert!(!name.trim().is_empty(), "{id} has no display name");
        }
    }

    #[test]
    fn rule_modes_round_trip_through_their_stored_form() {
        // These strings are a CHECK constraint in the schema; a mismatch fails
        // at write time rather than compile time.
        for mode in [RuleMode::Allow, RuleMode::Ignore] {
            assert_eq!(RuleMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(RuleMode::parse("maybe"), None);
    }
}
