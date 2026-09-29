use crate::completion::sequence::SequenceHistory;
use crate::completion::{CompletionProvider, CompletionWordSuppressions};
use crate::correction::LexicalSnapshot;
use crate::input::PhysicalKey;
use crate::language::{
    DictionaryEntry, LanguageId, builtin_language_pack, language_pack_from_entries,
    switch_keyboard_layout_text,
};
use crate::lexicon::UserLexicon;

use super::live_prefix::{
    LivePrefixDecision, LivePrefixDecisionTrace, LivePrefixKeepReason, LivePrefixLayoutPolicy,
    LivePrefixLayoutState, LivePrefixThresholds,
};

#[derive(Debug, Clone, Copy)]
enum ExpectedStep {
    Keep,
    Switch {
        replacement: &'static str,
        language: &'static str,
    },
    Reverse {
        replacement: &'static str,
        language: &'static str,
    },
}

#[derive(Debug, Clone, Copy)]
struct ScenarioStep {
    visible_prefix: &'static str,
    expected: ExpectedStep,
}

#[derive(Debug)]
struct ScenarioResult {
    unexpected_switches: usize,
    missed_switches: usize,
    unexpected_reverses: usize,
    missed_reverses: usize,
    flip_flops: usize,
}

impl ScenarioResult {
    fn passed(&self) -> bool {
        self.unexpected_switches == 0
            && self.missed_switches == 0
            && self.unexpected_reverses == 0
            && self.missed_reverses == 0
            && self.flip_flops == 0
    }

    fn score(&self) -> usize {
        self.unexpected_switches * 100
            + self.unexpected_reverses * 100
            + self.flip_flops * 50
            + self.missed_reverses * 20
            + self.missed_switches * 10
    }
}

fn provider_with_entries(
    english_words: Vec<(&'static str, u32)>,
    russian_words: Vec<(&'static str, u32)>,
) -> CompletionProvider {
    let en = language_pack_from_entries(
        LanguageId::try_new("en").unwrap(),
        english_words
            .into_iter()
            .map(|(word, frequency)| DictionaryEntry::try_new(word, frequency).unwrap())
            .collect(),
    )
    .unwrap();
    let ru = language_pack_from_entries(
        LanguageId::try_new("ru").unwrap(),
        russian_words
            .into_iter()
            .map(|(word, frequency)| DictionaryEntry::try_new(word, frequency).unwrap())
            .collect(),
    )
    .unwrap();
    CompletionProvider::with_word_suppressions(
        std::sync::Arc::new(
            LexicalSnapshot::try_new(vec![en, ru], UserLexicon::default()).unwrap(),
        ),
        std::sync::Arc::new(SequenceHistory::default()),
        std::sync::Arc::new(CompletionWordSuppressions::default()),
    )
}

fn realistic_provider() -> CompletionProvider {
    provider_with_entries(
        vec![
            ("hello", 1000),
            ("world", 900),
            ("weather", 850),
            ("data", 800),
            ("api", 780),
            ("project", 760),
            ("machine", 740),
            ("computer", 720),
        ],
        vec![
            ("время", 1000),
            ("времени", 950),
            ("временно", 900),
            ("как", 1200),
            ("привет", 1000),
            ("проект", 950),
            ("машина", 900),
            ("человек", 850),
            ("компьютер", 820),
        ],
    )
}

fn controlled_double_switch_provider() -> CompletionProvider {
    provider_with_entries(
        vec![
            ("hell", 1200),
            ("hello", 1000),
            ("data", 900),
            ("dhtvdata", 1100),
        ],
        vec![
            ("врем", 1200),
            ("время", 1000),
            ("времени", 900),
            ("привет", 1000),
            ("руддпривет", 1100),
        ],
    )
}

fn physical(text: &str) -> Vec<PhysicalKey> {
    text.chars().map(PhysicalKey::from_layout_symbol).collect()
}

fn assert_step(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
    state: &mut LivePrefixLayoutState,
    step: ScenarioStep,
    now_ms: i64,
) -> LivePrefixDecisionTrace {
    let was_active = state.is_active();
    let trace = policy.trace(
        provider,
        &[],
        step.visible_prefix,
        &physical(step.visible_prefix),
        now_ms,
        state,
    );
    match step.expected {
        ExpectedStep::Keep => {
            assert!(
                matches!(trace.decision, LivePrefixDecision::Keep { .. }),
                "expected keep for {:?}, got {:#?}",
                step.visible_prefix,
                trace
            );
        }
        ExpectedStep::Switch {
            replacement,
            language,
        } => {
            assert!(
                !was_active,
                "expected first switch from inactive state: {trace:#?}"
            );
            assert_replacement(&trace, replacement, language, step.visible_prefix);
            apply_trace_replacement(state, &trace);
        }
        ExpectedStep::Reverse {
            replacement,
            language,
        } => {
            assert!(was_active, "expected reverse from active state: {trace:#?}");
            assert_replacement(&trace, replacement, language, step.visible_prefix);
            apply_trace_replacement(state, &trace);
        }
    }
    trace
}

fn assert_replacement(
    trace: &LivePrefixDecisionTrace,
    expected_replacement: &str,
    expected_language: &str,
    source: &str,
) {
    let LivePrefixDecision::Replace(replacement) = &trace.decision else {
        panic!("{source:?} expected replacement {expected_replacement:?}, got {trace:#?}");
    };
    assert_eq!(
        replacement.replacement, expected_replacement,
        "source={source:?}"
    );
    assert_eq!(
        replacement.target_language.as_str(),
        expected_language,
        "source={source:?}"
    );
}

fn apply_trace_replacement(state: &mut LivePrefixLayoutState, trace: &LivePrefixDecisionTrace) {
    let LivePrefixDecision::Replace(replacement) = &trace.decision else {
        return;
    };
    *state = LivePrefixLayoutState::after_applied(replacement.target_language.clone());
}

fn run_required_scenario(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
    steps: &[ScenarioStep],
) {
    let mut state = LivePrefixLayoutState::default();
    for (index, step) in steps.iter().copied().enumerate() {
        assert_step(policy, provider, &mut state, step, 100 + index as i64);
    }
}

fn evaluate_scenario(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
    steps: &[ScenarioStep],
) -> ScenarioResult {
    let mut state = LivePrefixLayoutState::default();
    let mut result = ScenarioResult {
        unexpected_switches: 0,
        missed_switches: 0,
        unexpected_reverses: 0,
        missed_reverses: 0,
        flip_flops: 0,
    };
    let mut previous_language: Option<String> = None;
    let mut replacements = 0usize;

    for (index, step) in steps.iter().copied().enumerate() {
        let was_active = state.is_active();
        let trace = policy.trace(
            provider,
            &[],
            step.visible_prefix,
            &physical(step.visible_prefix),
            100 + index as i64,
            &state,
        );
        let actual = match &trace.decision {
            LivePrefixDecision::Keep { .. } => None,
            LivePrefixDecision::Replace(replacement) => Some(replacement.target_language.as_str()),
        };
        match (step.expected, actual, was_active) {
            (ExpectedStep::Keep, None, _) => {}
            (ExpectedStep::Keep, Some(_), false) => result.unexpected_switches += 1,
            (ExpectedStep::Keep, Some(_), true) => result.unexpected_reverses += 1,
            (ExpectedStep::Switch { language, .. }, Some(actual), false) if actual == language => {}
            (ExpectedStep::Switch { .. }, None, _) => result.missed_switches += 1,
            (ExpectedStep::Switch { .. }, Some(_), _) => result.unexpected_switches += 1,
            (ExpectedStep::Reverse { language, .. }, Some(actual), true) if actual == language => {}
            (ExpectedStep::Reverse { .. }, None, _) => result.missed_reverses += 1,
            (ExpectedStep::Reverse { .. }, Some(_), _) => result.unexpected_reverses += 1,
        }
        if let LivePrefixDecision::Replace(replacement) = &trace.decision {
            if previous_language.as_deref() == Some(replacement.target_language.as_str())
                && replacements > 0
            {
                result.flip_flops += 1;
            }
            previous_language = Some(replacement.target_language.as_str().to_owned());
            replacements += 1;
            apply_trace_replacement(&mut state, &trace);
        }
    }
    result
}

#[test]
fn certification_completion_suppression_does_not_change_live_prefix_evidence() {
    let en = language_pack_from_entries(
        LanguageId::try_new("en").unwrap(),
        vec![DictionaryEntry::try_new("hello", 1000).unwrap()],
    )
    .unwrap();
    let ru = language_pack_from_entries(
        LanguageId::try_new("ru").unwrap(),
        vec![DictionaryEntry::try_new("привет", 1000).unwrap()],
    )
    .unwrap();
    let snapshot = std::sync::Arc::new(
        LexicalSnapshot::try_new(vec![en, ru], UserLexicon::default()).unwrap(),
    );
    let sequences = std::sync::Arc::new(SequenceHistory::default());
    let visible = CompletionProvider::with_word_suppressions(
        std::sync::Arc::clone(&snapshot),
        std::sync::Arc::clone(&sequences),
        std::sync::Arc::new(CompletionWordSuppressions::default()),
    );
    let hidden = CompletionProvider::with_word_suppressions(
        snapshot,
        sequences,
        std::sync::Arc::new(CompletionWordSuppressions::from_normalized_words([
            "hello".to_owned()
        ])),
    );

    assert_eq!(
        visible.prefix_evidence(&[], "hell", 100, 3),
        hidden.prefix_evidence(&[], "hell", 100, 3),
        "completion-only suppression must not alter live layout evidence"
    );
    assert!(hidden.complete("hell", 3).is_empty());
}

#[test]
fn certification_live_prefix_realistic_switch_and_keep_corpus() {
    let provider = realistic_provider();
    let policy = LivePrefixLayoutPolicy::default();
    for (source, expected, language) in [
        ("dhtv", "врем", "ru"),
        ("ghbd", "прив", "ru"),
        ("ghjtrn", "проект", "ru"),
        ("vfibyf", "машина", "ru"),
        ("xtkj", "чело", "ru"),
        ("rjvg", "комп", "ru"),
        ("рудд", "hell", "en"),
        ("цщкд", "worl", "en"),
        ("цуфе", "weat", "en"),
        ("вфеф", "data", "en"),
    ] {
        let mut state = LivePrefixLayoutState::default();
        assert_step(
            policy,
            &provider,
            &mut state,
            ScenarioStep {
                visible_prefix: source,
                expected: ExpectedStep::Switch {
                    replacement: expected,
                    language,
                },
            },
            100,
        );
    }

    for source in [
        "hello",
        "hell",
        "weather",
        "weat",
        "data",
        "api",
        "project",
        "GPT5",
        "ETH",
        "SOL",
        "AA33",
        "foo_bar",
        "camelCase",
        "PascalCase",
        "user@example",
        "src/main",
        "C:\\Temp",
    ] {
        let trace = policy.trace(
            &provider,
            &[],
            source,
            &physical(source),
            100,
            &LivePrefixLayoutState::default(),
        );
        assert!(
            matches!(trace.decision, LivePrefixDecision::Keep { .. }),
            "{source:?} should stay in visible layout, got {trace:#?}"
        );
    }
}

#[test]
fn certification_live_prefix_controlled_double_switch_en_ru_en() {
    let provider = controlled_double_switch_provider();
    run_required_scenario(
        LivePrefixLayoutPolicy::default(),
        &provider,
        &[
            ScenarioStep {
                visible_prefix: "d",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "dh",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "dht",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "dhtv",
                expected: ExpectedStep::Switch {
                    replacement: "врем",
                    language: "ru",
                },
            },
            ScenarioStep {
                visible_prefix: "времв",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "времвф",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "времвфе",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "времвфеф",
                expected: ExpectedStep::Reverse {
                    replacement: "dhtvdata",
                    language: "en",
                },
            },
        ],
    );
}

#[test]
fn certification_live_prefix_controlled_double_switch_ru_en_ru() {
    let provider = controlled_double_switch_provider();
    run_required_scenario(
        LivePrefixLayoutPolicy::default(),
        &provider,
        &[
            ScenarioStep {
                visible_prefix: "р",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "ру",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "руд",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "рудд",
                expected: ExpectedStep::Switch {
                    replacement: "hell",
                    language: "en",
                },
            },
            ScenarioStep {
                visible_prefix: "hellg",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "hellghb",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "hellghbdt",
                expected: ExpectedStep::Keep,
            },
            ScenarioStep {
                visible_prefix: "hellghbdtn",
                expected: ExpectedStep::Reverse {
                    replacement: "руддпривет",
                    language: "ru",
                },
            },
        ],
    );
}

#[test]
fn certification_live_prefix_hysteresis_blocks_weak_reverse_and_allows_exact_reverse() {
    let provider = controlled_double_switch_provider();
    let policy = LivePrefixLayoutPolicy::default();
    let mut state = LivePrefixLayoutState::after_applied(LanguageId::try_new("ru").unwrap());

    let weak = policy.trace(&provider, &[], "времвф", &physical("времвф"), 120, &state);
    assert_eq!(
        weak.decision,
        LivePrefixDecision::Keep {
            reason: LivePrefixKeepReason::SwitchMarginTooSmall
        },
        "weak reverse should be blocked by hysteresis: {weak:#?}"
    );

    let strong = policy.trace(
        &provider,
        &[],
        "времвфеф",
        &physical("времвфеф"),
        130,
        &state,
    );
    assert_replacement(&strong, "dhtvdata", "en", "времвфеф");
    apply_trace_replacement(&mut state, &strong);

    let no_flip = policy.trace(
        &provider,
        &[],
        "dhtvdatax",
        &physical("dhtvdatax"),
        140,
        &state,
    );
    assert!(
        matches!(no_flip.decision, LivePrefixDecision::Keep { .. }),
        "reverse must not immediately flip back without strong opposite evidence: {no_flip:#?}"
    );
}

#[test]
fn certification_live_prefix_thresholds_pass_required_corpus() {
    let provider = controlled_double_switch_provider();
    let required = [
        ScenarioStep {
            visible_prefix: "dhtv",
            expected: ExpectedStep::Switch {
                replacement: "врем",
                language: "ru",
            },
        },
        ScenarioStep {
            visible_prefix: "времвф",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "времвфеф",
            expected: ExpectedStep::Reverse {
                replacement: "dhtvdata",
                language: "en",
            },
        },
    ];
    let result = evaluate_scenario(LivePrefixLayoutPolicy::default(), &provider, &required);
    assert!(result.passed(), "production thresholds failed: {result:#?}");
}

#[derive(Debug, Clone, Copy)]
struct CalibrationWord {
    word: &'static str,
    language: &'static str,
}

const ENGLISH_CALIBRATION_WORDS: [&str; 50] = [
    "work",
    "time",
    "life",
    "home",
    "help",
    "world",
    "house",
    "water",
    "people",
    "person",
    "child",
    "children",
    "friend",
    "family",
    "school",
    "language",
    "system",
    "change",
    "result",
    "value",
    "number",
    "simple",
    "current",
    "example",
    "problem",
    "program",
    "process",
    "support",
    "message",
    "history",
    "project",
    "computer",
    "keyboard",
    "weather",
    "working",
    "worked",
    "trying",
    "different",
    "important",
    "information",
    "application",
    "development",
    "performance",
    "correction",
    "completion",
    "dictionary",
    "environment",
    "configuration",
    "communication",
    "international",
];

const RUSSIAN_CALIBRATION_WORDS: [&str; 50] = [
    "время",
    "работа",
    "жизнь",
    "дома",
    "помощь",
    "привет",
    "погода",
    "данные",
    "проект",
    "система",
    "изменение",
    "язык",
    "компьютер",
    "клавиатура",
    "окно",
    "сообщение",
    "история",
    "текущий",
    "пример",
    "проблема",
    "программа",
    "процесс",
    "поддержка",
    "простой",
    "результат",
    "значение",
    "число",
    "люди",
    "человек",
    "дети",
    "друзья",
    "семья",
    "школа",
    "работает",
    "работал",
    "пытаюсь",
    "разные",
    "важный",
    "информация",
    "приложение",
    "разработка",
    "производительность",
    "исправление",
    "дополнение",
    "словарь",
    "окружение",
    "конфигурация",
    "коммуникация",
    "международный",
    "автоматический",
];

const TECHNICAL_KEEP_CASES: [&str; 16] = [
    "GPT5",
    "ETH",
    "SOL",
    "AA33",
    "foo_bar",
    "camelCase",
    "PascalCase",
    "user@example",
    "src/main",
    "C:\\Temp",
    "HTTP2",
    "UTF8",
    "api_v2",
    "hello.world",
    "foo/bar",
    "A1B2C3",
];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct WordCorpusMetrics {
    words: usize,
    false_switch_words: usize,
    wrong_target_switch_words: usize,
    missed_switch_words: usize,
    false_reverse_words: usize,
    missed_reverse_words: usize,
    technical_false_switches: usize,
    switch_delay_chars: usize,
    reverse_delay_chars: usize,
    max_switch_delay_chars: usize,
    max_reverse_delay_chars: usize,
}

impl WordCorpusMetrics {
    fn hard_errors(self) -> usize {
        self.false_switch_words
            + self.wrong_target_switch_words
            + self.false_reverse_words
            + self.technical_false_switches
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PrefixSwitchOutcome {
    None,
    CorrectAt(usize),
    WrongAt(usize),
}

fn builtin_calibration_provider() -> CompletionProvider {
    let english = builtin_language_pack(&LanguageId::try_new("en").unwrap())
        .unwrap()
        .expect("built-in English dictionary must exist");
    let russian = builtin_language_pack(&LanguageId::try_new("ru").unwrap())
        .unwrap()
        .expect("built-in Russian dictionary must exist");
    CompletionProvider::with_word_suppressions(
        std::sync::Arc::new(
            LexicalSnapshot::try_new(vec![english, russian], UserLexicon::default()).unwrap(),
        ),
        std::sync::Arc::new(SequenceHistory::default()),
        std::sync::Arc::new(CompletionWordSuppressions::default()),
    )
}

fn calibration_words() -> Vec<CalibrationWord> {
    let mut words = Vec::with_capacity(100);
    words.extend(
        ENGLISH_CALIBRATION_WORDS
            .iter()
            .copied()
            .map(|word| CalibrationWord {
                word,
                language: "en",
            }),
    );
    words.extend(
        RUSSIAN_CALIBRATION_WORDS
            .iter()
            .copied()
            .map(|word| CalibrationWord {
                word,
                language: "ru",
            }),
    );
    words
}

fn char_prefix(text: &str, chars: usize) -> String {
    text.chars().take(chars).collect()
}

fn transformed_spelling(
    provider: &CompletionProvider,
    word: CalibrationWord,
) -> (String, LanguageId) {
    let switched = switch_keyboard_layout_text(provider.lexical_snapshot().languages(), word.word)
        .unwrap_or_else(|| panic!("calibration word {:?} has no layout transform", word.word));
    assert_ne!(
        switched.target_language().as_str(),
        word.language,
        "layout transform must move {:?} to the opposite language",
        word.word
    );
    (
        switched.text().to_owned(),
        switched.target_language().clone(),
    )
}

fn first_switch_outcome(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
    visible: &str,
    intended: &str,
    intended_language: &str,
    state: &LivePrefixLayoutState,
) -> PrefixSwitchOutcome {
    let visible_chars = visible.chars().count();
    for prefix_chars in 4..=visible_chars {
        let visible_prefix = char_prefix(visible, prefix_chars);
        let trace = policy.trace(
            provider,
            &[],
            &visible_prefix,
            &physical(&visible_prefix),
            1_000 + prefix_chars as i64,
            state,
        );
        if let LivePrefixDecision::Replace(replacement) = trace.decision {
            let expected_prefix = char_prefix(intended, prefix_chars);
            if replacement.target_language.as_str() == intended_language
                && replacement.replacement == expected_prefix
            {
                return PrefixSwitchOutcome::CorrectAt(prefix_chars);
            }
            return PrefixSwitchOutcome::WrongAt(prefix_chars);
        }
    }
    PrefixSwitchOutcome::None
}

fn has_any_switch(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
    visible: &str,
    state: &LivePrefixLayoutState,
) -> bool {
    for prefix_chars in 4..=visible.chars().count() {
        let visible_prefix = char_prefix(visible, prefix_chars);
        let trace = policy.trace(
            provider,
            &[],
            &visible_prefix,
            &physical(&visible_prefix),
            2_000 + prefix_chars as i64,
            state,
        );
        if matches!(trace.decision, LivePrefixDecision::Replace(_)) {
            return true;
        }
    }
    false
}

fn evaluate_word_corpus(
    policy: LivePrefixLayoutPolicy,
    provider: &CompletionProvider,
) -> WordCorpusMetrics {
    let mut metrics = WordCorpusMetrics::default();
    for word in calibration_words() {
        metrics.words += 1;
        assert!(
            provider
                .prefix_evidence(&[], word.word, 0, 3)
                .has_exact_word,
            "calibration word {:?} must be an exact built-in dictionary word",
            word.word
        );
        assert!(
            word.word.chars().count() >= 4,
            "live-prefix calibration words must have at least four characters: {:?}",
            word.word
        );

        let (wrong_visible, opposite_language) = transformed_spelling(provider, word);
        let intended_language = LanguageId::try_new(word.language).unwrap();
        let inactive = LivePrefixLayoutState::default();

        if has_any_switch(policy, provider, word.word, &inactive) {
            metrics.false_switch_words += 1;
        }

        match first_switch_outcome(
            policy,
            provider,
            &wrong_visible,
            word.word,
            word.language,
            &inactive,
        ) {
            PrefixSwitchOutcome::CorrectAt(at) => {
                let delay = at.saturating_sub(4);
                metrics.switch_delay_chars += delay;
                metrics.max_switch_delay_chars = metrics.max_switch_delay_chars.max(delay);
            }
            PrefixSwitchOutcome::WrongAt(_) => metrics.wrong_target_switch_words += 1,
            PrefixSwitchOutcome::None => metrics.missed_switch_words += 1,
        }

        let stable_target_state = LivePrefixLayoutState::after_applied(intended_language);
        if has_any_switch(policy, provider, word.word, &stable_target_state) {
            metrics.false_reverse_words += 1;
        }

        let wrong_active_state = LivePrefixLayoutState::after_applied(opposite_language);
        match first_switch_outcome(
            policy,
            provider,
            &wrong_visible,
            word.word,
            word.language,
            &wrong_active_state,
        ) {
            PrefixSwitchOutcome::CorrectAt(at) => {
                let delay = at.saturating_sub(4);
                metrics.reverse_delay_chars += delay;
                metrics.max_reverse_delay_chars = metrics.max_reverse_delay_chars.max(delay);
            }
            PrefixSwitchOutcome::WrongAt(_) | PrefixSwitchOutcome::None => {
                metrics.missed_reverse_words += 1;
            }
        }
    }

    for technical in TECHNICAL_KEEP_CASES {
        if has_any_switch(
            policy,
            provider,
            technical,
            &LivePrefixLayoutState::default(),
        ) {
            metrics.technical_false_switches += 1;
        }
    }
    metrics
}

fn length_distribution() -> [usize; 5] {
    let mut buckets = [0usize; 5];
    for word in calibration_words() {
        match word.word.chars().count() {
            4 => buckets[0] += 1,
            5..=6 => buckets[1] += 1,
            7..=8 => buckets[2] += 1,
            9..=11 => buckets[3] += 1,
            _ => buckets[4] += 1,
        }
    }
    buckets
}

#[test]
fn certification_live_prefix_100_word_corpus_is_valid_and_default_thresholds_are_safe() {
    let provider = builtin_calibration_provider();
    let words = calibration_words();
    assert_eq!(words.len(), 100);
    assert_eq!(
        words.iter().filter(|word| word.language == "en").count(),
        50
    );
    assert_eq!(
        words.iter().filter(|word| word.language == "ru").count(),
        50
    );
    assert_eq!(
        length_distribution(),
        [10, 32, 27, 23, 8],
        "calibration corpus must retain broad word-length coverage"
    );

    let metrics = evaluate_word_corpus(LivePrefixLayoutPolicy::default(), &provider);
    assert_eq!(metrics.words, 100);
    assert_eq!(
        metrics.hard_errors(),
        0,
        "default live-prefix thresholds caused unsafe switches: {metrics:#?}"
    );
    assert_eq!(metrics.missed_switch_words, 0, "{metrics:#?}");
    assert_eq!(metrics.missed_reverse_words, 0, "{metrics:#?}");
    assert!(metrics.max_switch_delay_chars <= 1, "{metrics:#?}");
    assert!(metrics.max_reverse_delay_chars <= 1, "{metrics:#?}");
}

fn controlled_hysteresis_score(policy: LivePrefixLayoutPolicy) -> usize {
    let provider = controlled_double_switch_provider();
    let en_ru_en = [
        ScenarioStep {
            visible_prefix: "dhtv",
            expected: ExpectedStep::Switch {
                replacement: "врем",
                language: "ru",
            },
        },
        ScenarioStep {
            visible_prefix: "времв",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "времвф",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "времвфе",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "времвфеф",
            expected: ExpectedStep::Reverse {
                replacement: "dhtvdata",
                language: "en",
            },
        },
    ];
    let ru_en_ru = [
        ScenarioStep {
            visible_prefix: "рудд",
            expected: ExpectedStep::Switch {
                replacement: "hell",
                language: "en",
            },
        },
        ScenarioStep {
            visible_prefix: "hellg",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "hellghb",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "hellghbdt",
            expected: ExpectedStep::Keep,
        },
        ScenarioStep {
            visible_prefix: "hellghbdtn",
            expected: ExpectedStep::Reverse {
                replacement: "руддпривет",
                language: "ru",
            },
        },
    ];
    evaluate_scenario(policy, &provider, &en_ru_en).score()
        + evaluate_scenario(policy, &provider, &ru_en_ru).score()
}

fn print_false_switch_words(policy: LivePrefixLayoutPolicy, provider: &CompletionProvider) {
    for word in calibration_words() {
        for prefix_chars in 4..=word.word.chars().count() {
            let prefix = char_prefix(word.word, prefix_chars);
            let trace = policy.trace(
                provider,
                &[],
                &prefix,
                &physical(&prefix),
                5_000 + prefix_chars as i64,
                &LivePrefixLayoutState::default(),
            );
            if let LivePrefixDecision::Replace(replacement) = trace.decision {
                println!(
                    "FALSE_SWITCH word={:?} prefix={:?} -> {:?} lang={} same={:.3} cross={:.3} delta={:.3}",
                    word.word,
                    prefix,
                    replacement.replacement,
                    replacement.target_language.as_str(),
                    replacement.same_score,
                    replacement.cross_score,
                    replacement.cross_score - replacement.same_score
                );
                break;
            }
        }
    }
}

#[test]
#[ignore]
fn calibration_live_prefix_focused_forward_report() {
    let provider = builtin_calibration_provider();
    print_false_switch_words(LivePrefixLayoutPolicy::default(), &provider);
    let mut ranked = Vec::new();
    for min_score in [1.6, 1.8, 2.0, 2.2] {
        for switch_margin in [1.3, 1.4, 1.5, 1.6, 1.7] {
            for layout_penalty_scale in [1.75, 2.0, 2.25, 2.5] {
                let thresholds = LivePrefixThresholds {
                    min_score,
                    switch_margin,
                    reverse_margin: 2.7,
                    layout_penalty_scale,
                };
                let policy = LivePrefixLayoutPolicy::with_thresholds(thresholds);
                let metrics = evaluate_word_corpus(policy, &provider);
                let controlled_score = controlled_hysteresis_score(policy);
                let key = (
                    metrics.hard_errors(),
                    metrics.missed_switch_words,
                    metrics.max_switch_delay_chars,
                    metrics.switch_delay_chars,
                    controlled_score,
                );
                ranked.push((key, thresholds, metrics));
            }
        }
    }
    ranked.sort_by_key(|(key, _, _)| *key);
    for (rank, (key, thresholds, metrics)) in ranked.into_iter().take(20).enumerate() {
        println!(
            "FORWARD {} key={key:?} thresholds={thresholds:?} metrics={metrics:?}",
            rank + 1
        );
    }
}

#[test]
#[ignore]
fn calibration_live_prefix_margin_boundaries_report() {
    let provider = builtin_calibration_provider();
    for switch_margin in [
        1.2, 1.3, 1.4, 1.5, 1.6, 1.7, 1.8, 1.9, 2.0, 2.1, 2.2, 2.4, 2.6,
    ] {
        let thresholds = LivePrefixThresholds {
            min_score: 2.0,
            switch_margin,
            reverse_margin: 2.7,
            layout_penalty_scale: 2.0,
        };
        let policy = LivePrefixLayoutPolicy::with_thresholds(thresholds);
        println!(
            "SWITCH_BOUNDARY margin={switch_margin:.2} metrics={:?} controlled={}",
            evaluate_word_corpus(policy, &provider),
            controlled_hysteresis_score(policy)
        );
    }
    for effective_reverse_margin in [
        1.8, 2.0, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 3.0, 3.2, 3.4,
    ] {
        let thresholds = LivePrefixThresholds {
            min_score: 2.0,
            switch_margin: 1.7,
            reverse_margin: effective_reverse_margin,
            layout_penalty_scale: 2.0,
        };
        let policy = LivePrefixLayoutPolicy::with_thresholds(thresholds);
        println!(
            "REVERSE_BOUNDARY margin={effective_reverse_margin:.2} metrics={:?} controlled={}",
            evaluate_word_corpus(policy, &provider),
            controlled_hysteresis_score(policy)
        );
    }
}
