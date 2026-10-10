use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use crate::input::CompletedToken;
use crate::language::{
    KeyboardLayoutMap, LanguageId, LanguagePack, TextCasePattern, normalize_word,
};
use crate::lexicon::{IgnoredWords, MAX_INDEX_DELETIONS, UserLexicon, UserWord};

use super::{Confidence, CorrectionCandidate, CorrectionCandidateProvider, ReplacementText};
const REPEATED_DELETE_COST: f32 = 0.45;
const TRANSPOSE_COST: f32 = 0.75;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LexicalProviderError {
    NoLanguages,
}

#[derive(Debug, Clone)]
pub struct LexicalSnapshot {
    languages: Arc<[LanguagePack]>,
    user_lexicon: Arc<UserLexicon>,
    ignored_words: Arc<IgnoredWords>,
}

impl LexicalSnapshot {
    pub fn try_new(
        languages: Vec<LanguagePack>,
        user_lexicon: UserLexicon,
    ) -> Result<Self, LexicalProviderError> {
        Self::try_new_with_ignored(languages, user_lexicon, IgnoredWords::default())
    }

    pub fn try_new_with_ignored(
        languages: Vec<LanguagePack>,
        user_lexicon: UserLexicon,
        ignored_words: IgnoredWords,
    ) -> Result<Self, LexicalProviderError> {
        if languages.is_empty() {
            return Err(LexicalProviderError::NoLanguages);
        }
        Ok(Self {
            languages: languages.into(),
            user_lexicon: Arc::new(user_lexicon),
            ignored_words: Arc::new(ignored_words),
        })
    }

    pub fn languages(&self) -> &[LanguagePack] {
        &self.languages
    }

    pub fn user_lexicon(&self) -> &UserLexicon {
        &self.user_lexicon
    }

    pub fn ignored_words(&self) -> &IgnoredWords {
        &self.ignored_words
    }

    pub fn contains_normalized(&self, normalized: &str) -> bool {
        if self.ignored_words.contains_normalized(normalized) {
            return false;
        }
        self.user_lexicon.contains_normalized(normalized)
            || self
                .languages
                .iter()
                .any(|language| language.contains_normalized(normalized))
    }

    pub fn contains_system_normalized(&self, normalized: &str) -> bool {
        !self.ignored_words.contains_normalized(normalized)
            && self
                .languages
                .iter()
                .any(|language| language.contains_normalized(normalized))
    }

    pub(crate) fn with_user_lexicon(&self, user_lexicon: UserLexicon) -> Self {
        Self {
            languages: Arc::clone(&self.languages),
            user_lexicon: Arc::new(user_lexicon),
            ignored_words: Arc::clone(&self.ignored_words),
        }
    }

    pub(crate) fn with_lexical_overrides(
        &self,
        user_lexicon: UserLexicon,
        ignored_words: IgnoredWords,
    ) -> Self {
        Self {
            languages: Arc::clone(&self.languages),
            user_lexicon: Arc::new(user_lexicon),
            ignored_words: Arc::new(ignored_words),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LexicalCorrectionProvider {
    snapshot: Arc<LexicalSnapshot>,
}

impl LexicalCorrectionProvider {
    pub fn try_new(
        languages: Vec<LanguagePack>,
        user_lexicon: UserLexicon,
    ) -> Result<Self, LexicalProviderError> {
        Ok(Self {
            snapshot: Arc::new(LexicalSnapshot::try_new(languages, user_lexicon)?),
        })
    }

    pub fn from_snapshot(snapshot: Arc<LexicalSnapshot>) -> Self {
        Self { snapshot }
    }

    pub fn languages(&self) -> &[LanguagePack] {
        self.snapshot.languages()
    }

    pub fn user_lexicon(&self) -> &UserLexicon {
        self.snapshot.user_lexicon()
    }

    fn contains_normalized(&self, normalized: &str) -> bool {
        self.snapshot.contains_normalized(normalized)
    }

    fn contains_system_normalized(&self, normalized: &str) -> bool {
        self.snapshot.contains_system_normalized(normalized)
    }

    fn exact_cross_layout_interpretation(
        &self,
        token: &CompletedToken,
    ) -> Option<CrossLayoutInterpretation> {
        self.languages().iter().find_map(|language| {
            language.transforms().iter().find_map(|transform| {
                let transformed =
                    transform.transform_with_physical(token.text(), token.physical_keys())?;
                let view = LiteralWordView::from_transformed(&transformed)?;
                let normalized_core = normalize_word(&view.core);
                if self
                    .snapshot
                    .ignored_words()
                    .contains_normalized(&normalized_core)
                {
                    return None;
                }
                language.exact_entry(&normalized_core)?;
                Some(CrossLayoutInterpretation {
                    normalized_core,
                    target_language: language.id().clone(),
                })
            })
        })
    }

    pub fn canonical_learning_term(&self, token: &CompletedToken) -> String {
        if let Some(view) = LiteralWordView::from_token(token) {
            return view.core;
        }
        if token.text().chars().any(char::is_alphanumeric) {
            token.text().to_owned()
        } else {
            String::new()
        }
    }
}

impl CorrectionCandidateProvider for LexicalCorrectionProvider {
    fn candidates(&self, token: &CompletedToken) -> Vec<CorrectionCandidate> {
        let observed = token.text();
        let normalized = normalize_word(observed);
        if normalized.is_empty() {
            return Vec::new();
        }

        let exact_user_entry = (!self
            .snapshot
            .ignored_words()
            .contains_normalized(&normalized))
        .then(|| self.user_lexicon().exact(&normalized))
        .flatten();
        let exact_user_word = exact_user_entry.is_some();
        let exact_system_word = self.contains_system_normalized(&normalized);
        let exact_cross_layout = self.exact_cross_layout_interpretation(token);
        let has_exact_cross_layout = exact_cross_layout.is_some();
        let learned_function_word_alias = exact_user_entry.is_some_and(|entry| {
            learned_one_character_alias_can_yield_function_word(
                &normalized,
                exact_cross_layout.as_ref(),
                entry,
                self.user_lexicon(),
            )
        });
        let single_character_layout_ambiguity = normalized.chars().count() == 1
            && has_exact_cross_layout
            && ((!exact_user_word && exact_system_word) || learned_function_word_alias);

        let literal_view = LiteralWordView::from_token(token);
        let literal_core_is_valid = literal_view
            .as_ref()
            .is_some_and(|view| self.contains_normalized(&normalize_word(&view.core)));
        if (exact_user_word && !learned_function_word_alias)
            || (exact_system_word && !single_character_layout_ambiguity)
            || (literal_core_is_valid && !has_exact_cross_layout)
        {
            // Exact learned words remain authoritative, except for a one-character learned
            // alias whose opposite-layout interpretation is a short Russian function word
            // the user actually uses more often (for example learned `b` should still yield
            // `и` when `и` is the stronger user word). A one-character system spelling may
            // also be the physical spelling of an exact word in another enabled layout
            // (for example `z` -> `я`); unlike longer bilingual words there is not enough
            // lexical context to treat that system alias as decisive. Literal punctuation on
            // an otherwise valid word remains protected unless the complete physical token
            // has an exact cross-layout interpretation.
            return Vec::new();
        }

        let Some(literal_view) = literal_view else {
            return Vec::new();
        };
        let mut best_by_replacement: HashMap<String, RankedCandidate> = HashMap::new();

        for language in self.languages() {
            for variant in language_variants(language, token, &literal_view) {
                if let Some(entry) = language.exact_entry(&variant.text)
                    && !self
                        .snapshot
                        .ignored_words()
                        .contains_normalized(entry.word())
                {
                    consider_language_entry(
                        &mut best_by_replacement,
                        observed,
                        language,
                        &variant,
                        &entry,
                    );
                }
                for entry in language.candidate_entries(&variant.text, MAX_INDEX_DELETIONS) {
                    if entry.word() == variant.text
                        || self
                            .snapshot
                            .ignored_words()
                            .contains_normalized(entry.word())
                    {
                        continue;
                    }
                    consider_language_entry(
                        &mut best_by_replacement,
                        observed,
                        language,
                        &variant,
                        entry,
                    );
                }
            }
        }

        for variant in user_variants(self.languages(), token, &literal_view) {
            let max_edit_cost = max_edit_cost(variant.text.chars().count());
            for entry in self
                .user_lexicon()
                .candidate_entries(&variant.text, MAX_INDEX_DELETIONS)
            {
                if self
                    .snapshot
                    .ignored_words()
                    .contains_normalized(entry.normalized_term())
                    || !digit_signature_matches(&variant.text, entry.normalized_term())
                {
                    continue;
                }
                let edit_cost = weighted_damerau_cost(&variant.text, entry.normalized_term());
                if edit_cost > max_edit_cost + f32::EPSILON {
                    continue;
                }
                if edit_cost == 0.0 && variant.layout_penalty == 0.0 {
                    continue;
                }

                let corrected = variant.case_pattern.apply(entry.term());
                let replacement = format!(
                    "{}{}{}",
                    variant.literal_prefix, corrected, variant.literal_suffix
                );
                if replacement == observed {
                    continue;
                }

                let confidence = candidate_confidence(
                    edit_cost,
                    variant.layout_penalty,
                    entry.normalized_term().chars().count(),
                    entry.use_count(),
                    self.user_lexicon().max_use_count(),
                );
                insert_ranked_candidate(
                    &mut best_by_replacement,
                    RankedCandidate {
                        replacement,
                        confidence,
                        edit_cost,
                        layout_penalty: variant.layout_penalty,
                        frequency: entry.use_count(),
                        target_language: variant.target_language.clone(),
                    },
                );
            }
        }

        let mut ranked: Vec<_> = best_by_replacement.into_values().collect();
        ranked.sort_by(|left, right| {
            right
                .confidence
                .value()
                .partial_cmp(&left.confidence.value())
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.replacement.cmp(&right.replacement))
        });

        ranked
            .into_iter()
            .filter_map(|candidate| {
                let replacement = ReplacementText::try_new(candidate.replacement).ok()?;
                let features = crate::correction::CorrectionFeatures::new(
                    candidate.edit_cost > f32::EPSILON,
                    candidate.target_language.is_some(),
                );
                Some(CorrectionCandidate::classified(
                    replacement,
                    candidate.confidence,
                    candidate.target_language,
                    features,
                ))
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
struct CrossLayoutInterpretation {
    normalized_core: String,
    target_language: LanguageId,
}

#[derive(Debug, Clone)]
struct LiteralWordView {
    prefix: String,
    core: String,
    suffix: String,
}

impl LiteralWordView {
    fn from_token(token: &CompletedToken) -> Option<Self> {
        let characters: Vec<_> = token.text().chars().collect();
        let physical_keys = token.physical_keys();
        if characters.len() != physical_keys.len() {
            return None;
        }
        Self::split(&characters, |index, character| {
            is_deferred_literal_punctuation(character, physical_keys[index])
        })
    }

    fn from_transformed(text: &str) -> Option<Self> {
        let characters: Vec<_> = text.chars().collect();
        Self::split(&characters, |_index, character| {
            !character.is_alphanumeric()
        })
    }

    fn split(characters: &[char], is_literal_edge: impl Fn(usize, char) -> bool) -> Option<Self> {
        let mut start = 0;
        while start < characters.len() && is_literal_edge(start, characters[start]) {
            start += 1;
        }

        let mut end = characters.len();
        while end > start && is_literal_edge(end - 1, characters[end - 1]) {
            end -= 1;
        }

        if start == end {
            return None;
        }

        Some(Self {
            prefix: characters[..start].iter().collect(),
            core: characters[start..end].iter().collect(),
            suffix: characters[end..].iter().collect(),
        })
    }
}

fn is_deferred_literal_punctuation(
    character: char,
    physical_key: crate::input::PhysicalKey,
) -> bool {
    physical_key.is_layout_ambiguous() && !character.is_alphanumeric()
}

fn learned_one_character_alias_can_yield_function_word(
    normalized_observed: &str,
    interpretation: Option<&CrossLayoutInterpretation>,
    learned: &UserWord,
    user_lexicon: &UserLexicon,
) -> bool {
    if normalized_observed.chars().count() != 1 {
        return false;
    }
    let Some(interpretation) = interpretation else {
        return false;
    };
    if !is_short_function_word(
        &interpretation.target_language,
        &interpretation.normalized_core,
    ) {
        return false;
    }
    let target_use_count = user_lexicon
        .exact(&interpretation.normalized_core)
        .map(UserWord::use_count)
        .unwrap_or(0);
    target_use_count > learned.use_count()
}

fn is_short_function_word(language: &LanguageId, normalized_word: &str) -> bool {
    matches!(
        (language.as_str(), normalized_word),
        ("ru", "а" | "в" | "и" | "к" | "о" | "с" | "у")
    )
}

#[derive(Debug, Clone)]
struct ObservedVariant {
    text: String,
    layout_penalty: f32,
    literal_prefix: String,
    literal_suffix: String,
    case_pattern: TextCasePattern,
    target_language: Option<LanguageId>,
}

fn language_variants(
    language: &LanguagePack,
    token: &CompletedToken,
    literal_view: &LiteralWordView,
) -> Vec<ObservedVariant> {
    observed_variants(
        language
            .transforms()
            .iter()
            .map(|transform| (language.id(), transform)),
        token,
        literal_view,
    )
}

fn user_variants(
    languages: &[LanguagePack],
    token: &CompletedToken,
    literal_view: &LiteralWordView,
) -> Vec<ObservedVariant> {
    observed_variants(
        languages.iter().flat_map(|language| {
            language
                .transforms()
                .iter()
                .map(move |transform| (language.id(), transform))
        }),
        token,
        literal_view,
    )
}

fn observed_variants<'a>(
    transforms: impl IntoIterator<Item = (&'a LanguageId, &'a KeyboardLayoutMap)>,
    token: &CompletedToken,
    literal_view: &LiteralWordView,
) -> Vec<ObservedVariant> {
    let mut variants = Vec::new();
    push_variant(
        &mut variants,
        ObservedVariant {
            text: normalize_word(&literal_view.core),
            layout_penalty: 0.0,
            literal_prefix: literal_view.prefix.clone(),
            literal_suffix: literal_view.suffix.clone(),
            case_pattern: TextCasePattern::detect(&literal_view.core),
            target_language: None,
        },
    );

    for (target_language, transform) in transforms {
        if let Some(transformed) =
            transform.transform_with_physical(token.text(), token.physical_keys())
            && let Some(transformed_view) = LiteralWordView::from_transformed(&transformed)
        {
            push_variant(
                &mut variants,
                ObservedVariant {
                    text: normalize_word(&transformed_view.core),
                    layout_penalty: transform.penalty(),
                    literal_prefix: transformed_view.prefix,
                    literal_suffix: transformed_view.suffix,
                    case_pattern: TextCasePattern::detect(&transformed_view.core),
                    target_language: Some(target_language.clone()),
                },
            );
        }
    }

    variants.sort_by(|left, right| {
        left.layout_penalty
            .partial_cmp(&right.layout_penalty)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.text.cmp(&right.text))
            .then_with(|| left.literal_prefix.cmp(&right.literal_prefix))
            .then_with(|| left.literal_suffix.cmp(&right.literal_suffix))
    });
    variants
}

fn push_variant(variants: &mut Vec<ObservedVariant>, candidate: ObservedVariant) {
    if let Some(existing) = variants.iter_mut().find(|existing| {
        existing.text == candidate.text
            && existing.literal_prefix == candidate.literal_prefix
            && existing.literal_suffix == candidate.literal_suffix
            && existing.case_pattern == candidate.case_pattern
    }) {
        existing.layout_penalty = existing.layout_penalty.min(candidate.layout_penalty);
    } else {
        variants.push(candidate);
    }
}

fn consider_language_entry(
    candidates: &mut HashMap<String, RankedCandidate>,
    observed: &str,
    language: &LanguagePack,
    variant: &ObservedVariant,
    entry: &crate::language::DictionaryEntry,
) {
    if !digit_signature_matches(&variant.text, entry.word()) {
        return;
    }
    let edit_cost = weighted_damerau_cost(&variant.text, entry.word());
    if edit_cost > max_edit_cost(variant.text.chars().count()) + f32::EPSILON
        || (edit_cost == 0.0 && variant.layout_penalty == 0.0)
    {
        return;
    }

    let corrected = variant.case_pattern.apply(entry.word());
    let replacement = format!(
        "{}{}{}",
        variant.literal_prefix, corrected, variant.literal_suffix
    );
    if replacement == observed {
        return;
    }

    let confidence = candidate_confidence(
        edit_cost,
        variant.layout_penalty,
        entry.word().chars().count(),
        entry.frequency(),
        language.max_frequency(),
    );
    insert_ranked_candidate(
        candidates,
        RankedCandidate {
            replacement,
            confidence,
            edit_cost,
            layout_penalty: variant.layout_penalty,
            frequency: entry.frequency(),
            target_language: variant.target_language.clone(),
        },
    );
}

fn digit_signature_matches(observed: &str, target: &str) -> bool {
    observed
        .chars()
        .filter(|character| character.is_ascii_digit())
        .eq(target
            .chars()
            .filter(|character| character.is_ascii_digit()))
}

#[derive(Debug, Clone)]
struct RankedCandidate {
    replacement: String,
    confidence: Confidence,
    edit_cost: f32,
    layout_penalty: f32,
    frequency: u32,
    target_language: Option<LanguageId>,
}

impl RankedCandidate {
    fn is_better_than(&self, other: &Self) -> bool {
        self.confidence.value() > other.confidence.value()
            || (self.confidence.value() == other.confidence.value()
                && (self.edit_cost + self.layout_penalty < other.edit_cost + other.layout_penalty
                    || (self.edit_cost + self.layout_penalty
                        == other.edit_cost + other.layout_penalty
                        && self.frequency > other.frequency)))
    }
}

fn insert_ranked_candidate(
    candidates: &mut HashMap<String, RankedCandidate>,
    ranked: RankedCandidate,
) {
    candidates
        .entry(ranked.replacement.clone())
        .and_modify(|current| {
            if ranked.is_better_than(current) {
                *current = ranked.clone();
            }
        })
        .or_insert(ranked);
}

fn max_edit_cost(observed_chars: usize) -> f32 {
    match observed_chars {
        0..=2 => 0.0,
        3 => 0.80,
        4 => 1.0,
        _ => 2.0,
    }
}

fn candidate_confidence(
    edit_cost: f32,
    layout_penalty: f32,
    target_chars: usize,
    weight: u32,
    max_weight: u32,
) -> Confidence {
    let total_cost = edit_cost + layout_penalty;
    let length_scale = target_chars.max(3) as f32 + 1.5;
    let quality = (1.0 - total_cost / length_scale).clamp(0.0, 1.0);
    let weight_ratio = (weight as f32 / max_weight.max(1) as f32).sqrt();
    let score = (quality * 0.96 + weight_ratio * 0.04).clamp(0.0, 0.999);
    Confidence::try_new(score).expect("lexical confidence is clamped to the typed range")
}

pub(super) fn weighted_damerau_cost(observed: &str, target: &str) -> f32 {
    let source: Vec<char> = observed.chars().collect();
    let target: Vec<char> = target.chars().collect();
    let mut distance = vec![vec![0.0_f32; target.len() + 1]; source.len() + 1];

    for index in 1..=source.len() {
        distance[index][0] = distance[index - 1][0] + source_delete_cost(&source, index - 1);
    }
    for index in 1..=target.len() {
        distance[0][index] = distance[0][index - 1] + 1.0;
    }

    for source_index in 1..=source.len() {
        for target_index in 1..=target.len() {
            let substitution_cost = if source[source_index - 1] == target[target_index - 1] {
                0.0
            } else {
                1.0
            };

            let deletion = distance[source_index - 1][target_index]
                + source_delete_cost(&source, source_index - 1);
            let insertion = distance[source_index][target_index - 1] + 1.0;
            let substitution = distance[source_index - 1][target_index - 1] + substitution_cost;
            let mut best = deletion.min(insertion).min(substitution);

            if source_index > 1
                && target_index > 1
                && source[source_index - 1] == target[target_index - 2]
                && source[source_index - 2] == target[target_index - 1]
            {
                best = best.min(distance[source_index - 2][target_index - 2] + TRANSPOSE_COST);
            }

            distance[source_index][target_index] = best;
        }
    }

    distance[source.len()][target.len()]
}

fn source_delete_cost(source: &[char], index: usize) -> f32 {
    let repeats_left = index > 0 && source[index] == source[index - 1];
    let repeats_right = index + 1 < source.len() && source[index] == source[index + 1];
    if repeats_left || repeats_right {
        REPEATED_DELETE_COST
    } else {
        1.0
    }
}
