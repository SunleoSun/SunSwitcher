use super::{UserLexicon, UserWord};

#[test]
fn certification_user_lexicon_supports_known_word_typo_and_prefix_views_from_one_snapshot() {
    let lexicon = UserLexicon::try_new(vec![
        UserWord::try_new("QuantileEntryStrategy", 4, 100).unwrap(),
        UserWord::try_new("QuantileEntryStrategy1", 1, 200).unwrap(),
    ])
    .unwrap();

    assert!(lexicon.contains_normalized("quantileentrystrategy"));
    assert_eq!(
        lexicon.exact("quantileentrystrategy1").unwrap().term(),
        "QuantileEntryStrategy1"
    );
    assert!(
        lexicon
            .candidate_entries("quanntileentrysrtategy", 2)
            .iter()
            .any(|entry| entry.term() == "QuantileEntryStrategy")
    );

    let completions = lexicon.prefix_matches("QuantileEntry", 2);
    assert_eq!(
        completions
            .iter()
            .map(|entry| entry.term())
            .collect::<Vec<_>>(),
        ["QuantileEntryStrategy1", "QuantileEntryStrategy"]
    );
}
