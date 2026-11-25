use crate::adaptive::{AdaptiveCorrectionDirective, AdaptiveLexicalRuntime};
use crate::correction::Confidence;
use crate::input::InputEvent;
use crate::persistence::Database;

#[test]
fn certification_application_exclusion_preserves_globals_and_reports_full_input_suppression() {
    use crate::application_policy::ApplicationIdentity;
    use crate::persistence::AppSettings;
    use crate::settings::SettingsStore;
    use sun_observations::certifications::*;

    let mut c = certification("application exclusion is a full input suppression");
    let settings = SettingsStore::from_database(Database::open_in_memory().unwrap()).unwrap();
    let excluded = ApplicationIdentity::try_new("C:/Apps/excluded.exe").unwrap();
    let ordinary = ApplicationIdentity::try_new("C:/Apps/ordinary.exe").unwrap();

    settings
        .set_application_suppressed(&excluded, true)
        .unwrap();

    c.check_fact_expect(
        "excluded foreground bypasses SunSwitcher input",
        settings.input_suppressed(Some(&excluded)),
        true,
    );
    c.check_fact_expect(
        "ordinary foreground remains enabled",
        settings.input_suppressed(Some(&ordinary)),
        false,
    );
    c.check_fact_expect(
        "application exclusion preserves global settings",
        settings.load().unwrap(),
        AppSettings::default(),
    );

    settings
        .set_application_suppressed(&excluded, false)
        .unwrap();
    c.check_fact_expect(
        "removal restores global input behavior",
        settings.input_suppressed(Some(&excluded)),
        false,
    );
    c.assert_certify();
}

#[test]
fn certification_foreground_transitions_restore_and_suppress_live_session_automatic_replacements() {
    use crate::application_policy::ApplicationIdentity;
    use crate::replacement::ReplacementOutcome;
    use crate::settings::SettingsStore;
    use sun_observations::certifications::*;
    let mut c =
        certification("foreground transitions change policy in an existing correction session");
    let settings = SettingsStore::from_database(Database::open_in_memory().unwrap()).unwrap();
    let suppressed = ApplicationIdentity::try_new("C:/Apps/suppressed.exe").unwrap();
    let ordinary = ApplicationIdentity::try_new("C:/Apps/ordinary.exe").unwrap();
    settings
        .set_application_suppressed(&suppressed, true)
        .unwrap();
    let runtime = AdaptiveLexicalRuntime::start(
        Database::open_in_memory().unwrap(),
        Confidence::try_new(0.80).unwrap(),
    )
    .unwrap();
    let mut correction = runtime.session();
    for (foreground, token, should_replace) in [
        (Some(&ordinary), "дял", true),
        (Some(&suppressed), "hlelo", false),
        (None, "ghbdtn", false),
        (Some(&ordinary), "руддщ", true),
        (Some(&suppressed), "дял", false),
    ] {
        let policy = settings.correction_policy(foreground);
        correction
            .process_with_policy(InputEvent::Invalidate, 100, policy)
            .unwrap();
        for character in token.chars() {
            c.check_fact_expect(
                "partial token does not replace",
                correction
                    .process_with_policy(InputEvent::character(character), 100, policy)
                    .unwrap(),
                AdaptiveCorrectionDirective::Pass,
            );
        }
        let result = correction
            .process_with_policy(InputEvent::character(' '), 100, policy)
            .unwrap();
        c.check_fact_expect(
            "foreground permission controls boundary replacement",
            matches!(result, AdaptiveCorrectionDirective::Replace(_)),
            should_replace,
        );
        if matches!(result, AdaptiveCorrectionDirective::Replace(_)) {
            correction
                .replacement_outcome(ReplacementOutcome::Applied)
                .unwrap();
        }
        if !should_replace {
            let mut live = runtime.live_session();
            for character in "rfr".chars() {
                c.check_fact_expect(
                    "suppressed or unresolved foreground denies live-prefix replacement",
                    live.process_with_policy(InputEvent::character(character), 100, policy)
                        .unwrap(),
                    AdaptiveCorrectionDirective::Pass,
                );
            }
        }
    }
    settings
        .update(|current| current.with_enable_autocorrections(false))
        .unwrap();
    let policy = settings.correction_policy(Some(&ordinary));
    correction
        .process_with_policy(InputEvent::Invalidate, 200, policy)
        .unwrap();
    for character in "helllo ".chars() {
        c.check_fact_expect(
            "global autocorrections OFF remains authoritative",
            correction
                .process_with_policy(InputEvent::character(character), 200, policy)
                .unwrap(),
            AdaptiveCorrectionDirective::Pass,
        );
    }
    c.assert_certify();
}
