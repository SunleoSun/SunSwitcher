use super::{UserLexicon, UserLexiconError, UserTerm, UserTermProtection};

fn term(value: &str, protection: UserTermProtection, use_count: u32, last_used: i64) -> UserTerm {
    UserTerm::try_new(value, protection, use_count, last_used).unwrap()
}

#[test]
fn user_term_contract_rejects_unrepresentable_values() {
    assert_eq!(
        UserTerm::try_new("", UserTermProtection::Normal, 1, 0),
        Err(UserLexiconError::InvalidTerm)
    );
    assert_eq!(
        UserTerm::try_new("two words", UserTermProtection::Normal, 1, 0),
        Err(UserLexiconError::InvalidTerm)
    );
    assert_eq!(
        UserTerm::try_new("word", UserTermProtection::Normal, 0, 0),
        Err(UserLexiconError::InvalidUseCount)
    );
}

#[test]
fn normalized_duplicates_are_rejected_instead_of_competing() {
    let result = UserLexicon::try_new(vec![
        term("QuantileEntryStrategy", UserTermProtection::Normal, 1, 10),
        term(
            "quantileentrystrategy",
            UserTermProtection::Protected,
            1,
            20,
        ),
    ]);
    assert!(matches!(
        result,
        Err(UserLexiconError::DuplicateNormalizedTerm(_))
    ));
}

#[test]
fn delete_index_finds_typo_candidates_without_losing_canonical_spelling() {
    let lexicon = UserLexicon::try_new(vec![term(
        "QuantileEntryStrategy",
        UserTermProtection::Normal,
        3,
        100,
    )])
    .unwrap();

    let candidates = lexicon.candidate_entries("quanntileentrysrtategy", 2);
    assert!(candidates.iter().any(|candidate| {
        candidate.term() == "QuantileEntryStrategy"
            && candidate.normalized_term() == "quantileentrystrategy"
    }));
}

#[test]
fn prefix_matches_rank_recency_before_frequency() {
    let lexicon = UserLexicon::try_new(vec![
        term("QuantileEntryStrategy", UserTermProtection::Normal, 20, 100),
        term(
            "QuantileEntryStrategy1",
            UserTermProtection::Protected,
            1,
            200,
        ),
    ])
    .unwrap();

    let matches = lexicon.prefix_matches("QuantileEntry", 10);
    assert_eq!(matches[0].term(), "QuantileEntryStrategy1");
    assert_eq!(matches[1].term(), "QuantileEntryStrategy");
}
