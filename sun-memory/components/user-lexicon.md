---
description: Explains canonical learned user words, immutable UserLexicon indexes, ranking metadata, and equal runtime correction semantics with system dictionaries; read before changing learned vocabulary, Undo learning, or future autocomplete ranking.
---
# User Lexicon

## Purpose

The user lexicon is the language-neutral dictionary learned from user behavior. It is a second persisted vocabulary source beside the shipped system dictionaries, not an exception/protection list. A word accepted through normal kept input, trusted clipboard text, or a successful Undo has the same validity semantics once it is in the user dictionary.

## Ownership boundary

SQLite `user_words` rows are the persistent authority for learned vocabulary. `src/lexicon/user_lexicon.rs` owns the immutable runtime `UserLexicon` and typed `UserWord` contract. A user word stores stable canonical spelling/casing from its first accepted observation, normalized identity, use count, and last-use time; later case variants advance usage/recency without rewriting that canonical stored spelling. Runtime lookup remains case-insensitive through normalized identity, while correction/completion presentation adapts simple casing to the user's currently observed text so an early capitalized observation cannot force capitalization in later lower-case prose. Mixed/camel identifiers keep their canonical internal casing except for the exact prefix already typed by the user. There is no `Protected` flag or language field. Language/layout interpretation belongs to `LanguagePack` transforms. Those transforms may preserve layout-neutral non-alphabetic characters such as digits, allowing a language-neutral learned identifier (`AA33`) to remain a normal `UserLexicon` target after an opposite-layout interpretation (`ФФ33` -> `AA33`) without adding language metadata to `user_words`. For one-character ambiguity, an explicit learned `user_words` row remains the authority over any system-dictionary cross-layout interpretation; only a system-only one-character spelling may yield to an exact opposite-layout system word.

Built-in system vocabulary and learned `user_words` stay in separate authorities because their ownership and metadata differ: system vocabulary is immutable language-scoped FST data, while learned rows are mutable language-neutral and usage-backed SQLite state. This separation has no keyboard-hot-path penalty because both are loaded into immutable in-memory exact/delete indexes before correction. `UserLexicon` shares its exact/delete indexes through `Arc`; normalized identities are shared `Arc<str>` values between `UserWord` rows and the exact index, immutable spellings use compact boxed strings, and the shared typo delete index compiles deletion-form keys into an in-memory FST whose values encode single candidates inline and use compact boxed `u32` postings only for shared forms. An existing word's use-count/recency update clones only the lightweight `Arc<UserWord>` entry vector and reuses those indexes, while genuinely new/deleted membership triggers the canonical full index rebuild.

## Runtime contract

`LexicalCorrectionProvider` treats exact system and exact user words as equally valid: either suppresses automatic correction. Both system and user words also contribute typo candidates through the same correction engine and scoring path. Completed-token typo candidates preserve the ordered ASCII-digit signature of the observed token, so learned identifier siblings such as `sun1` and `sun3` remain distinct user intent instead of correcting one numeric identity into another; alphabetic typos remain correctable when the digits are unchanged. Candidate identity is normalized/case-insensitive; when a learned user-word typo is replaced, the canonical `UserWord` supplies spelling while `TextCasePattern` reapplies the observed lower/upper/title pattern. This prevents a stored capitalized form from rewriting later lower-case use while retaining one canonical normalized word.

`UserLexicon::prefix_matches` exposes learned words for future completion. Matching is normalized; ranking prefers more recent use, then higher use count, then deterministic spelling. Future autocomplete should combine system and user vocabulary views rather than create another word store.

## Learning semantics

A completed typed token is learned only after the correction engine decides `Keep`; a source token that is auto-corrected is not learned. Clipboard tokens are also run through the same correction decision and confidence threshold before admission. If a copied token would be replaced, its erroneous source spelling is not added to `user_words`; the correction target already belongs to the system or user vocabulary that produced the candidate. Successfully applied autocomplete acceptance re-observes accepted learned words through the same `LearningClient::observe_typed_token` path, so choosing a user-word completion advances its `use_count`/recency just like typing that word manually. An uncertain/failed completion effect advances no user evidence, and accepted system-dictionary words remain nonduplicated because normal admission still rejects system vocabulary as new `user_words` rows.

A successful Undo is explicit acceptance of the restored original. After external text restoration succeeds, `commit_correction_undo` atomically marks the correction undone and inserts/updates that spelling in `user_words`. There is no stronger protection tier: dictionary membership itself is the acceptance signal. Pause with no text selection keeps this Undo behavior. Pause with an existing selected word is the explicit removal path: it queues normalized deletion from `user_words` and refreshes the immutable live `UserLexicon` snapshot. Completion sequence rows remain historical evidence, but a sequence may surface only words still valid in the current lexical snapshot; deleting a custom user word therefore prevents old `text_history` from re-offering that removed spelling. Correction history does not act as a blacklist, and restarting SunSwitcher does not add, remove, or reinterpret persisted user words.

## High-value anchors

- `src/lexicon/user_lexicon.rs`: user-word contract, exact/delete/prefix indexes and ranking.
- `src/persistence/database.rs`: `user_words` schema, upsert, load, and Undo persistence.
- `src/correction/lexical_provider.rs`: equal exact validity and candidate arbitration across system/user vocabularies.
- `src/adaptive/adaptive_runtime.rs`: typed and copied-text admission plus live snapshot refresh.

## Validation

Run lexicon tests/certification, persistence fresh-schema/reopen tests, adaptive learning/Undo certifications, and lexical-provider certification. Changes affecting correction thresholds must verify clipboard admission uses the same threshold as typed correction.

## Related memory

- `main/core-project-principles`
- `components/persistence`
- `components/adaptive-correction`
- `components/clipboard-learning`