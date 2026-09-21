use super::{UserLexicon, UserLexiconError, UserWord};

fn word(value: &str, use_count: u32, last_used: i64) -> UserWord {
    UserWord::try_new(value, use_count, last_used).unwrap()
}

#[test]
fn user_word_contract_rejects_unrepresentable_values() {
    assert_eq!(
        UserWord::try_new("", 1, 0),
        Err(UserLexiconError::InvalidTerm)
    );
    assert_eq!(
        UserWord::try_new("two words", 1, 0),
        Err(UserLexiconError::InvalidTerm)
    );
    assert_eq!(
        UserWord::try_new("word", 0, 0),
        Err(UserLexiconError::InvalidUseCount)
    );
}

#[test]
fn normalized_duplicates_are_rejected_instead_of_competing() {
    let result = UserLexicon::try_new(vec![
        word("QuantileEntryStrategy", 1, 10),
        word("quantileentrystrategy", 1, 20),
    ]);
    assert!(matches!(
        result,
        Err(UserLexiconError::DuplicateNormalizedTerm(_))
    ));
}

#[test]
fn delete_index_finds_typo_candidates_without_losing_canonical_spelling() {
    let lexicon = UserLexicon::try_new(vec![word("QuantileEntryStrategy", 3, 100)]).unwrap();

    let candidates = lexicon.candidate_entries("quanntileentrysrtategy", 2);
    assert!(candidates.iter().any(|candidate| {
        candidate.term() == "QuantileEntryStrategy"
            && candidate.normalized_term() == "quantileentrystrategy"
    }));
}

#[test]
fn prefix_matches_rank_recency_before_frequency() {
    let lexicon = UserLexicon::try_new(vec![
        word("QuantileEntryStrategy", 20, 100),
        word("QuantileEntryStrategy1", 1, 200),
    ])
    .unwrap();

    let matches = lexicon.prefix_matches("QuantileEntry", 10);
    assert_eq!(matches[0].term(), "QuantileEntryStrategy1");
    assert_eq!(matches[1].term(), "QuantileEntryStrategy");
}
