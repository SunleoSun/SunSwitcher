use crate::adaptive::{AdaptiveCorrectionDirective, AdaptiveLexicalRuntime};
use crate::completion::{CompletionApplyOutcome, CompletionCommand, CompletionCommandResult};
use crate::correction::Confidence;
use crate::input::InputEvent;
use crate::persistence::Database;

#[test]
fn certification_application_suppression_preserves_live_completion_navigation_and_acceptance() {
    use sun_observations::certifications::*;
    let mut c = certification(
        "certification_application_suppression_preserves_live_completion_navigation_and_acceptance",
    );
    use crate::application_policy::ApplicationIdentity;
    use crate::persistence::AppSettings;
    use crate::settings::SettingsStore;
    let settings = SettingsStore::from_database(Database::open_in_memory().unwrap()).unwrap();
    let application = ApplicationIdentity::try_new("C:/Apps/example.exe").unwrap();
    settings
        .set_application_suppressed(&application, true)
        .unwrap();
    let database = Database::open_in_memory().unwrap();
    database
        .record_user_word("AcceptPrediction902", 10)
        .unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.80).unwrap()).unwrap();
    let mut correction = runtime.live_session();
    let mut completion = runtime.completion_session();
    for character in "AcceptPred".chars() {
        let event = InputEvent::character(character);

        completion.process_event(event, 100).unwrap();
        c.check_fact_expect(
            "automatic live replacement suppressed",
            correction
                .process_with_policy(event, 100, settings.correction_policy(Some(&application)))
                .unwrap(),
            AdaptiveCorrectionDirective::Pass,
        );
    }
    c.check_fact_expect(
        "completion session remains active",
        completion.is_active(),
        true,
    );
    c.check_fact_expect(
        "completion suggestions remain available",
        completion.suggestions()[0].text(),
        "AcceptPrediction902",
    );
    c.check_fact_expect(
        "next completion navigation consumed",
        completion.command(CompletionCommand::Next).unwrap(),
        CompletionCommandResult::Consumed,
    );
    c.check_fact_expect(
        "previous completion navigation consumed",
        completion.command(CompletionCommand::Previous).unwrap(),
        CompletionCommandResult::Consumed,
    );
    c.check_fact_expect(
        "explicit completion acceptance returns suffix",
        completion.command(CompletionCommand::Accept).unwrap(),
        CompletionCommandResult::AcceptSuffix("iction902".to_owned()),
    );
    completion
        .suffix_acceptance_outcome(CompletionApplyOutcome::Applied, 200)
        .unwrap();
    c.check_fact_expect(
        "application suppression preserves globals",
        settings.load().unwrap(),
        AppSettings::default(),
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
