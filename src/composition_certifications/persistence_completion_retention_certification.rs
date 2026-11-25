use crate::adaptive::AdaptiveLexicalRuntime;
use crate::correction::Confidence;
use crate::input::InputEvent;
use crate::persistence::Database;
use sun_observations::certifications::*;

fn prefix_suggestions(runtime: &AdaptiveLexicalRuntime, now_ms: i64) -> Vec<String> {
    let mut session = runtime.completion_session();
    for character in "hel".chars() {
        session
            .process_event(InputEvent::character(character), now_ms)
            .unwrap();
    }
    session
        .suggestions()
        .iter()
        .map(|r| r.text().to_owned())
        .collect()
}

#[test]
fn certification_maintenance_rebases_completion_after_expiring_base_and_overlay_evidence() {
    let mut c =
        certification("background retention removes expired base and overlay completion evidence");
    let now_ms = 1_736_899_200_123;
    let database = Database::open_in_memory().unwrap();
    database.record_text_history("hello world", 10).unwrap();
    database.record_text_history("hello there", now_ms).unwrap();
    let runtime =
        AdaptiveLexicalRuntime::start(database, Confidence::try_new(0.8).unwrap()).unwrap();
    runtime.learning().observe_text("hello world", 11).unwrap();
    runtime.flush().unwrap();
    c.check_fact_expect(
        "old evidence initially available",
        prefix_suggestions(&runtime, now_ms).contains(&"hello world".to_owned()),
        true,
    );
    let report = runtime.learning().maintain(now_ms).unwrap();
    let after = prefix_suggestions(&runtime, now_ms);
    c.check_fact_expect(
        "worker reports persistent expiration",
        report.text_history_deleted,
        1,
    );
    c.check_fact_expect(
        "expired sequence absent from RAM projection",
        after.contains(&"hello world".to_owned()),
        false,
    );
    c.check_fact_expect(
        "fresh canonical evidence remains",
        after.contains(&"hello there".to_owned()),
        true,
    );
    c.check_fact_expect(
        "maintenance rejection reaches caller",
        runtime.learning().maintain(i64::MAX).is_err(),
        true,
    );
    c.check_fact_expect(
        "rejection leaves published projection unchanged",
        prefix_suggestions(&runtime, now_ms),
        after,
    );
    c.assert_certify();
}
