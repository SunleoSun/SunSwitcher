use crate::completion::CompletionProvider;
use crate::input::PhysicalKey;
use crate::language::{LanguageId, normalize_word};

const LIVE_MIN_PREFIX_CHARS: usize = 4;
const LIVE_MAX_PREFIX_CHARS: usize = 48;
const LIVE_WORD_EVIDENCE_LIMIT: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LivePrefixThresholds {
    pub(super) min_score: f32,
    pub(super) switch_margin: f32,
    pub(super) reverse_margin: f32,
    pub(super) layout_penalty_scale: f32,
}

impl Default for LivePrefixThresholds {
    fn default() -> Self {
        Self {
            min_score: 2.0,
            switch_margin: 1.7,
            reverse_margin: 2.7,
            layout_penalty_scale: 2.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LivePrefixReplacement {
    pub(super) original_visible: String,
    pub(super) replacement: String,
    pub(super) target_language: LanguageId,
    pub(super) same_score: f32,
    pub(super) cross_score: f32,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub(super) struct LivePrefixLayoutState {
    current_target_language: Option<LanguageId>,
}

impl LivePrefixLayoutState {
    pub(super) fn after_applied(target_language: LanguageId) -> Self {
        Self {
            current_target_language: Some(target_language),
        }
    }

    pub(super) fn clear(&mut self) {
        self.current_target_language = None;
    }

    pub(super) fn current_target_language(&self) -> Option<&LanguageId> {
        self.current_target_language.as_ref()
    }

    pub(super) fn is_active(&self) -> bool {
        self.current_target_language.is_some()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LivePrefixKeepReason {
    TooShort,
    TooLong,
    PhysicalKeyMismatch,
    TechnicalToken,
    StrongExactSameLayoutWord,
    NoCrossLayoutCandidate,
    CrossScoreTooLow,
    SwitchMarginTooSmall,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) enum LivePrefixDecision {
    Keep { reason: LivePrefixKeepReason },
    Replace(LivePrefixReplacement),
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LivePrefixCandidateTrace {
    pub(super) target_language: LanguageId,
    pub(super) replacement: String,
    pub(super) raw_score: f32,
    pub(super) adjusted_score: f32,
    pub(super) delta: f32,
    pub(super) required_margin: f32,
    pub(super) accepted: bool,
    pub(super) rejected_reason: Option<LivePrefixKeepReason>,
}

#[derive(Debug, Clone, PartialEq)]
pub(super) struct LivePrefixDecisionTrace {
    pub(super) visible_prefix: String,
    pub(super) state_language: Option<LanguageId>,
    pub(super) same_score: f32,
    pub(super) same_exact: bool,
    pub(super) candidates: Vec<LivePrefixCandidateTrace>,
    pub(super) decision: LivePrefixDecision,
}

impl LivePrefixDecisionTrace {
    fn keep(
        visible_prefix: &str,
        state: &LivePrefixLayoutState,
        reason: LivePrefixKeepReason,
    ) -> Self {
        Self {
            visible_prefix: visible_prefix.to_owned(),
            state_language: state.current_target_language().cloned(),
            same_score: 0.0,
            same_exact: false,
            candidates: Vec::new(),
            decision: LivePrefixDecision::Keep { reason },
        }
    }

    fn replacement(&self) -> Option<LivePrefixReplacement> {
        match &self.decision {
            LivePrefixDecision::Replace(replacement) => Some(replacement.clone()),
            LivePrefixDecision::Keep { .. } => None,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq)]
pub(super) struct LivePrefixLayoutPolicy {
    thresholds: LivePrefixThresholds,
}

impl LivePrefixLayoutPolicy {
    #[cfg(test)]
    pub(super) fn with_thresholds(thresholds: LivePrefixThresholds) -> Self {
        Self { thresholds }
    }

    pub(super) fn decide(
        &self,
        provider: &CompletionProvider,
        context_tokens: &[String],
        visible_prefix: &str,
        physical_keys: &[PhysicalKey],
        now_ms: i64,
        state: &LivePrefixLayoutState,
    ) -> Option<LivePrefixReplacement> {
        self.decide_internal(
            provider,
            context_tokens,
            visible_prefix,
            physical_keys,
            now_ms,
            state,
        )
        .replacement()
    }

    #[cfg(test)]
    pub(super) fn trace(
        &self,
        provider: &CompletionProvider,
        context_tokens: &[String],
        visible_prefix: &str,
        physical_keys: &[PhysicalKey],
        now_ms: i64,
        state: &LivePrefixLayoutState,
    ) -> LivePrefixDecisionTrace {
        self.decide_internal(
            provider,
            context_tokens,
            visible_prefix,
            physical_keys,
            now_ms,
            state,
        )
    }

    fn decide_internal(
        &self,
        provider: &CompletionProvider,
        context_tokens: &[String],
        visible_prefix: &str,
        physical_keys: &[PhysicalKey],
        now_ms: i64,
        state: &LivePrefixLayoutState,
    ) -> LivePrefixDecisionTrace {
        let prefix_chars = visible_prefix.chars().count();
        if prefix_chars < LIVE_MIN_PREFIX_CHARS {
            return LivePrefixDecisionTrace::keep(
                visible_prefix,
                state,
                LivePrefixKeepReason::TooShort,
            );
        }
        if prefix_chars > LIVE_MAX_PREFIX_CHARS {
            return LivePrefixDecisionTrace::keep(
                visible_prefix,
                state,
                LivePrefixKeepReason::TooLong,
            );
        }
        if prefix_chars != physical_keys.len() {
            return LivePrefixDecisionTrace::keep(
                visible_prefix,
                state,
                LivePrefixKeepReason::PhysicalKeyMismatch,
            );
        }
        if looks_like_technical_token(visible_prefix, physical_keys) {
            return LivePrefixDecisionTrace::keep(
                visible_prefix,
                state,
                LivePrefixKeepReason::TechnicalToken,
            );
        }

        let context_refs = context_tokens
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>();
        let same_evidence = provider.prefix_evidence(
            &context_refs,
            visible_prefix,
            now_ms,
            LIVE_WORD_EVIDENCE_LIMIT,
        );
        let same_score = same_evidence.score();

        let mut trace = LivePrefixDecisionTrace {
            visible_prefix: visible_prefix.to_owned(),
            state_language: state.current_target_language().cloned(),
            same_score,
            same_exact: same_evidence.has_exact_word,
            candidates: Vec::new(),
            decision: LivePrefixDecision::Keep {
                reason: LivePrefixKeepReason::NoCrossLayoutCandidate,
            },
        };

        if !state.is_active()
            && same_evidence.has_exact_word
            && same_score >= self.thresholds.min_score
        {
            trace.decision = LivePrefixDecision::Keep {
                reason: LivePrefixKeepReason::StrongExactSameLayoutWord,
            };
            return trace;
        }

        let required_margin = if state.is_active() {
            self.thresholds.reverse_margin
        } else {
            self.thresholds.switch_margin
        };
        let mut best: Option<LivePrefixReplacement> = None;

        for language in provider.lexical_snapshot().languages() {
            if state
                .current_target_language()
                .is_some_and(|current| current == language.id())
            {
                continue;
            }
            for transform in language.transforms() {
                let Some(transformed) =
                    transform.transform_with_physical(visible_prefix, physical_keys)
                else {
                    continue;
                };
                if transformed == visible_prefix || normalize_word(&transformed).is_empty() {
                    continue;
                }
                let evidence = provider.prefix_evidence(
                    &context_refs,
                    &transformed,
                    now_ms,
                    LIVE_WORD_EVIDENCE_LIMIT,
                );
                let raw_score = evidence.score();
                let adjusted_score =
                    raw_score - transform.penalty() * self.thresholds.layout_penalty_scale;
                let delta = adjusted_score - same_score;
                let rejected_reason = if adjusted_score < self.thresholds.min_score {
                    Some(LivePrefixKeepReason::CrossScoreTooLow)
                } else if delta < required_margin {
                    Some(LivePrefixKeepReason::SwitchMarginTooSmall)
                } else {
                    None
                };
                let candidate_trace = LivePrefixCandidateTrace {
                    target_language: language.id().clone(),
                    replacement: transformed.clone(),
                    raw_score,
                    adjusted_score,
                    delta,
                    required_margin,
                    accepted: rejected_reason.is_none(),
                    rejected_reason,
                };
                trace.candidates.push(candidate_trace);
                if rejected_reason.is_some() {
                    continue;
                }

                let candidate = LivePrefixReplacement {
                    original_visible: visible_prefix.to_owned(),
                    replacement: transformed,
                    target_language: language.id().clone(),
                    same_score,
                    cross_score: adjusted_score,
                };
                if best.as_ref().is_none_or(|current| {
                    candidate.cross_score - candidate.same_score
                        > current.cross_score - current.same_score
                }) {
                    best = Some(candidate);
                }
            }
        }

        if let Some(replacement) = best {
            trace.decision = LivePrefixDecision::Replace(replacement);
            return trace;
        }

        trace.decision = LivePrefixDecision::Keep {
            reason: if trace.candidates.iter().any(|candidate| {
                candidate.rejected_reason == Some(LivePrefixKeepReason::SwitchMarginTooSmall)
            }) {
                LivePrefixKeepReason::SwitchMarginTooSmall
            } else if trace.candidates.iter().any(|candidate| {
                candidate.rejected_reason == Some(LivePrefixKeepReason::CrossScoreTooLow)
            }) {
                LivePrefixKeepReason::CrossScoreTooLow
            } else {
                LivePrefixKeepReason::NoCrossLayoutCandidate
            },
        };
        trace
    }
}

fn looks_like_technical_token(text: &str, physical_keys: &[PhysicalKey]) -> bool {
    if text.contains('_') || text.contains('@') || text.contains('\\') || text.contains('/') {
        return true;
    }
    if text.contains(":") && !physical_keys.iter().any(|key| key.is_layout_ambiguous()) {
        return true;
    }

    let mut has_digit = false;
    let mut has_upper = false;
    let mut has_lower = false;
    let mut alphabetic = 0usize;
    for character in text.chars() {
        if character.is_ascii_digit() {
            has_digit = true;
        }
        if character.is_alphabetic() {
            alphabetic += 1;
            has_upper |= character.is_uppercase();
            has_lower |= character.is_lowercase();
        }
    }

    if has_digit && alphabetic > 0 {
        return true;
    }
    if has_upper && !has_lower && alphabetic <= 6 {
        return true;
    }
    has_upper && has_lower && text.chars().next().is_some_and(char::is_lowercase)
}

#[cfg(test)]
mod tests {
    use crate::completion::sequence::SequenceHistory;
    use crate::completion::{CompletionProvider, CompletionWordSuppressions};
    use crate::correction::LexicalSnapshot;
    use crate::input::PhysicalKey;
    use crate::language::{DictionaryEntry, LanguageId, language_pack_from_entries};
    use crate::lexicon::UserLexicon;

    use super::{
        LivePrefixDecision, LivePrefixKeepReason, LivePrefixLayoutPolicy, LivePrefixLayoutState,
    };

    fn provider() -> CompletionProvider {
        let en = language_pack_from_entries(
            LanguageId::try_new("en").unwrap(),
            vec![
                DictionaryEntry::try_new("hello", 1000).unwrap(),
                DictionaryEntry::try_new("world", 800).unwrap(),
                DictionaryEntry::try_new("weather", 700).unwrap(),
                DictionaryEntry::try_new("data", 650).unwrap(),
                DictionaryEntry::try_new("api", 700).unwrap(),
                DictionaryEntry::try_new("project", 600).unwrap(),
            ],
        )
        .unwrap();
        let ru = language_pack_from_entries(
            LanguageId::try_new("ru").unwrap(),
            vec![
                DictionaryEntry::try_new("время", 1000).unwrap(),
                DictionaryEntry::try_new("времени", 900).unwrap(),
                DictionaryEntry::try_new("временно", 700).unwrap(),
                DictionaryEntry::try_new("как", 1200).unwrap(),
                DictionaryEntry::try_new("привет", 1000).unwrap(),
                DictionaryEntry::try_new("проект", 950).unwrap(),
                DictionaryEntry::try_new("машина", 850).unwrap(),
                DictionaryEntry::try_new("человек", 800).unwrap(),
                DictionaryEntry::try_new("компьютер", 750).unwrap(),
            ],
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

    fn physical(text: &str) -> Vec<PhysicalKey> {
        text.chars().map(PhysicalKey::from_layout_symbol).collect()
    }

    #[test]
    fn certification_live_prefix_score_switches_obvious_wrong_layout_prefixes() {
        let provider = provider();
        let decision = LivePrefixLayoutPolicy::default()
            .decide(
                &provider,
                &[],
                "dhtv",
                &physical("dhtv"),
                100,
                &LivePrefixLayoutState::default(),
            )
            .expect("dhtv should resolve to Russian врем prefix");
        assert_eq!(decision.replacement, "врем");
        assert_eq!(decision.target_language.as_str(), "ru");
        assert!(decision.cross_score > decision.same_score);
    }

    #[test]
    fn certification_live_prefix_score_preserves_strong_same_layout_words_and_technical_tokens() {
        let provider = provider();
        for source in ["hello", "AA33", "api"] {
            assert!(
                LivePrefixLayoutPolicy::default()
                    .decide(
                        &provider,
                        &[],
                        source,
                        &physical(source),
                        100,
                        &LivePrefixLayoutState::default(),
                    )
                    .is_none(),
                "{source:?} should stay in visible layout"
            );
        }
    }

    #[test]
    fn certification_live_prefix_score_includes_shifted_layout_punctuation() {
        let provider = provider();
        let mut keys = physical("rfr&");
        keys[3] = PhysicalKey::Digit7;
        let decision = LivePrefixLayoutPolicy::default()
            .decide(
                &provider,
                &[],
                "rfr&",
                &keys,
                100,
                &LivePrefixLayoutState::default(),
            )
            .expect("rfr& should resolve to как?");
        assert_eq!(decision.replacement, "как?");
        assert_eq!(decision.target_language.as_str(), "ru");
    }

    #[test]
    fn certification_live_prefix_trace_uses_typed_keep_reasons() {
        let provider = provider();
        let trace = LivePrefixLayoutPolicy::default().trace(
            &provider,
            &[],
            "GPT5",
            &physical("GPT5"),
            100,
            &LivePrefixLayoutState::default(),
        );
        assert_eq!(
            trace.decision,
            LivePrefixDecision::Keep {
                reason: LivePrefixKeepReason::TechnicalToken
            }
        );
    }
}
