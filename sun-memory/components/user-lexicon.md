---
description: Explains canonical user vocabulary, immutable UserLexicon indexes, protection/recency semantics, and correction integration; read before changing learned words, Undo protection, technical-identifier correction, or future autocomplete ranking.
---
# User Lexicon

## Purpose

The user lexicon represents local vocabulary learned or explicitly accepted by the user, including technical identifiers such as `QuantileEntryStrategy`. It is distinct from natural-language dictionaries because it preserves the user's canonical spelling/casing and carries usage/protection metadata needed by correction and future completion.

## Ownership boundary

SQLite `user_terms` rows are the persistent authority. User terms are intentionally language-neutral; language/layout interpretation belongs to `LanguagePack` transforms rather than a persisted user-term language field. `src/lexicon/user_lexicon.rs` owns the immutable runtime `UserLexicon` and typed `UserTerm`/`UserTermProtection` contracts. `src/lexicon/delete_index.rs` owns the shared delete-form candidate index used by both system language packs and the user lexicon.

`src/correction/lexical_provider.rs` consumes the runtime lexicon but does not own user-term persistence or ranking metadata. Future completion should consume the same snapshot rather than creating another user-word store.

## Runtime contracts

Normalized identity is lowercase-normalized while `UserTerm::term` preserves canonical display/replacement spelling. Exact user terms are known words and therefore suppress speculative automatic correction. Typo candidate discovery uses the same delete-index neighborhood and weighted edit scoring as language dictionaries, but replacement text preserves the stored user spelling instead of forcing natural-language case patterns.

`UserLexicon::prefix_matches` exposes the same snapshot for autocomplete-style prefix lookup. Matching is case-normalized; current ordering prefers more recent use, then higher use count, then protected status, with deterministic spelling order as the final tie-break.

## Protection semantics

`UserTermProtection::Protected` represents explicit user intent such as undoing an incorrect correction. Persistence makes protection sticky. A protected term remains a known exact term and normal later observations cannot silently replace its canonical spelling. Normal learned terms are also considered valid exact user vocabulary; protection records stronger intent for persistence and future ranking/policy decisions.

## Correction integration

`LexicalCorrectionProvider` receives both enabled `LanguagePack` snapshots and one `UserLexicon`. It checks exact user/system vocabulary before generating candidates. User candidates can also consume the configured language layout transforms, so the user lexicon remains language-neutral while wrong-layout interpretation stays owned by language data. Correction never queries SQLite.

## High-value navigation anchors

- `src/lexicon/user_lexicon.rs`: typed user terms, exact/delete lookup, prefix matching, ranking metadata.
- `src/lexicon/delete_index.rs`: shared delete-form candidate discovery.
- `src/persistence/database.rs`: canonical record/upsert and snapshot loading.
- `src/correction/lexical_provider.rs`: known-word protection and user-term typo candidates.
- `src/lexicon/user_lexicon_certification.rs`: runtime lexicon behavioral contract.
- `src/correction/lexical_provider_certification.rs`: end-to-end correction behavior with user terms.

## Safe change points

Change persistence semantics in `Database`, runtime lookup/ranking in `UserLexicon`, and correction arbitration/scoring in `LexicalCorrectionProvider`. Do not duplicate any of these policies in UI, clipboard capture, or Windows hook code. Undo uses the persistence two-phase contract: restore the prepared original text first, then commit protection and refresh the derived runtime snapshot.

## Validation

Run lexicon tests/certification for lookup/ranking behavior, persistence tests/certification for canonical upsert/reopen behavior, and lexical-provider certification for actual typo correction and exact-term protection.

## Revisit when

Revisit this entry when user-term normalization, protection semantics, prefix ranking, correction scoring, snapshot refresh, or completion consumption changes.

## Related memory

- `main/core-project-principles`
- `components/persistence`
- `components/adaptive-correction`

## Source inspection still required

Inspect current scoring thresholds and persistence SQL before changing ranking or correction policy. Typed technical-identifier learning and the reusable text-observation path are owned by `AdaptiveLexicalRuntime`; immediate typed-word Undo restoration is wired through the Windows runtime, while broader UI Undo and clipboard-listener wiring still require source inspection when implemented.