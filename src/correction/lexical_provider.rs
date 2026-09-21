use std::cmp::Ordering;
use std::collections::HashMap;
use std::sync::Arc;

use crate::input::CompletedToken;
use crate::language::{KeyboardLayoutMap, LanguagePack, normalize_word};
use crate::lexicon::{MAX_INDEX_DELETIONS, UserLexicon};

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
}

impl LexicalSnapshot {
    pub fn try_new(
        languages: Vec<LanguagePack>,
        user_lexicon: UserLexicon,
    ) -> Result<Self, LexicalProviderError> {
        if languages.is_empty() {
            return Err(LexicalProviderError::NoLanguages);
        }
        Ok(Self {
            languages: languages.into(),
            user_lexicon: Arc::new(user_lexicon),
        })
    }

    pub fn languages(&self) -> &[LanguagePack] {
        &self.languages
    }

    pub fn user_lexicon(&self) -> &UserLexicon {
        &self.user_lexicon
    }

    pub(crate) fn with_user_lexicon(&self, user_lexicon: UserLexicon) -> Self {
        Self {
            languages: Arc::clone(&self.languages),
            user_lexicon: Arc::new(user_lexicon),
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
        self.user_lexicon().contains_normalized(normalized)
            || self
                .languages()
                .iter()
                .any(|language| language.contains_normalized(normalized))
    }
}

impl CorrectionCandidateProvider for LexicalCorrectionProvider {
    fn candidates(&self, token: &CompletedToken) -> Vec<CorrectionCandidate> {
        let observed = token.text();
        let normalized = normalize_word(observed);
        if normalized.is_empty() {
            return Vec::new();
        }

        let literal_view = LiteralWordView::from_token(token);
        if self.contains_normalized(&normalized)
            || literal_view
                .as_ref()
                .is_some_and(|view| self.contains_normalized(&normalize_word(&view.core)))
        {
            // A literal word that is already valid in any configured language wins over
            // speculative cross-layout interpretation. This is especially important for
            // ordinary text followed by deferred punctuation such as "hello,".
            return Vec::new();
        }

        let Some(literal_view) = literal_view else {
            return Vec::new();
        };
        let mut best_by_replacement: HashMap<String, RankedCandidate> = HashMap::new();

        for language in self.languages() {
            for variant in language_variants(language, token, &literal_view) {
                let max_edit_cost = max_edit_cost(variant.text.chars().count());
                for entry in language.candidate_entries(&variant.text, MAX_INDEX_DELETIONS) {
                    let edit_cost = weighted_damerau_cost(&variant.text, entry.word());
                    if edit_cost > max_edit_cost + f32::EPSILON {
                        continue;
                    }
                    if edit_cost == 0.0 && variant.layout_penalty == 0.0 {
                        continue;
                    }

                    let corrected = variant.case_pattern.apply(entry.word());
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
                        entry.word().chars().count(),
                        entry.frequency(),
                        language.max_frequency(),
                    );
                    insert_ranked_candidate(
                        &mut best_by_replacement,
                        RankedCandidate {
                            replacement,
                            confidence,
                            edit_cost,
                            layout_penalty: variant.layout_penalty,
                            frequency: entry.frequency(),
                        },
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
                let edit_cost = weighted_damerau_cost(&variant.text, entry.normalized_term());
                if edit_cost > max_edit_cost + f32::EPSILON {
                    continue;
                }
                if edit_cost == 0.0 && variant.layout_penalty == 0.0 {
                    continue;
                }

                let replacement = format!(
                    "{}{}{}",
                    variant.literal_prefix,
                    entry.term(),
                    variant.literal_suffix
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
                Some(CorrectionCandidate::new(replacement, candidate.confidence))
            })
            .collect()
    }
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

#[derive(Debug, Clone)]
struct ObservedVariant {
    text: String,
    layout_penalty: f32,
    literal_prefix: String,
    literal_suffix: String,
    case_pattern: CasePattern,
}

fn language_variants(
    language: &LanguagePack,
    token: &CompletedToken,
    literal_view: &LiteralWordView,
) -> Vec<ObservedVariant> {
    observed_variants(language.transforms().iter(), token, literal_view)
}

fn user_variants(
    languages: &[LanguagePack],
    token: &CompletedToken,
    literal_view: &LiteralWordView,
) -> Vec<ObservedVariant> {
    observed_variants(
        languages
            .iter()
            .flat_map(|language| language.transforms().iter()),
        token,
        literal_view,
    )
}

fn observed_variants<'a>(
    transforms: impl IntoIterator<Item = &'a KeyboardLayoutMap>,
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
            case_pattern: CasePattern::detect(&literal_view.core),
        },
    );

    for transform in transforms {
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
                    case_pattern: CasePattern::detect(&transformed_view.core),
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

#[derive(Debug, Clone)]
struct RankedCandidate {
    replacement: String,
    confidence: Confidence,
    edit_cost: f32,
    layout_penalty: f32,
    frequency: u32,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CasePattern {
    Lower,
    Upper,
    Title,
    Mixed,
}

impl CasePattern {
    fn detect(text: &str) -> Self {
        let letters: Vec<char> = text
            .chars()
            .filter(|character| character.is_alphabetic())
            .collect();
        if letters.is_empty() || letters.iter().all(|character| character.is_lowercase()) {
            return Self::Lower;
        }
        if letters.iter().all(|character| character.is_uppercase()) {
            return Self::Upper;
        }
        if letters[0].is_uppercase()
            && letters[1..]
                .iter()
                .all(|character| character.is_lowercase())
        {
            return Self::Title;
        }
        Self::Mixed
    }

    fn apply(self, canonical: &str) -> String {
        match self {
            Self::Lower | Self::Mixed => canonical.to_owned(),
            Self::Upper => canonical.chars().flat_map(char::to_uppercase).collect(),
            Self::Title => {
                let mut characters = canonical.chars();
                let Some(first) = characters.next() else {
                    return String::new();
                };
                first
                    .to_uppercase()
                    .chain(characters.flat_map(char::to_lowercase))
                    .collect()
            }
        }
    }
}
