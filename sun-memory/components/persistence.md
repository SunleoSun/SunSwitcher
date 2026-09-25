---
description: Maps SunSwitcher's canonical SQLite ownership, fresh-schema bootstrap, system/user dictionary separation, runtime snapshot loading, and persistence validation; read before changing schema, settings, dictionaries, user words, clipboard storage, or DB-backed runtime state.
---
# SunSwitcher Persistence

## Purpose

One local SQLite database owns persistent application state. Keyboard/correction/completion hot paths consume validated in-memory snapshots and never query SQLite.

## Ownership boundary

`src/persistence/database.rs` owns database opening, the canonical bootstrap schema, `PRAGMA user_version`, typed persistence APIs, stored-row validation, and conversion into runtime snapshots. It does not own lexical policy, keyboard transforms, clipboard OS capture, or UI behavior.

System vocabulary is stored in language-scoped `dictionary_words`; learned vocabulary is stored separately in language-neutral `user_words`. The separation reflects different canonical metadata/lifecycles and does not affect hot-path performance because both become in-memory indexes before use. `user_words` carries canonical spelling, normalized identity, use count, and last-use time only; there is no protection flag.

## Schema evolution

The current schema version is 3. Version 1 remains the bootstrap baseline, version 2 adds the `completion_hidden_words` table, and version 3 removes legacy grave/backtick pollution through ordered forward migrations. Existing older databases are upgraded before use. `PRAGMA user_version` is the compatibility gate, and databases newer than the supported schema fail closed. Future persistent changes must continue as ordered forward migrations rather than rewriting older baselines.

Russian and English system surface forms/frequencies are canonical DB rows; valid inflections are exact dictionary entries rather than suffix heuristics. Language keyboard transforms remain in `src/language/`. `AppSettings` owns the clipboard history limit and Undo hotkey.

## User-word persistence

Schema v3 removes legacy `user_words` and `text_history` rows containing grave/backtick pollution created by the old deferred-layout learning bug; that cleanup is a one-time schema migration, not restart behavior. New typed learning strips such lexical-edge punctuation before persistence. `Database::record_user_word` owns user-word upsert semantics: normalized identity is unique, use count accumulates, last-use time never moves backward, and the first accepted canonical spelling remains stable across later case variants. `Database::delete_user_word` is the explicit normalized deletion path used by Pause on a selected word. `Database::load_user_lexicon` validates normalized spelling and typed counters before constructing the runtime snapshot. Runtime startup loads persisted `user_words` as-is and does not reconcile them against correction history. `Database::record_text_history` and `Database::load_text_history` own the durable upsert/rebuild boundary for canonical multi-word sequence n-grams from both observed text and physical typing; `delete_text_history` removes an explicitly rejected learned sequence. Clipboard/text observation may emit all contiguous 2-5 word n-grams from one canonical payload; rolling typed learning emits only suffix n-grams ending at the newly resolved word. `completion_hidden_words` is the independent persistent authority for single-word autocomplete suppression. `hide_completion_word` stores normalized identity without mutating `dictionary_words` or `user_words`, and `load_hidden_completion_words` validates/rebuilds the RAM suppression snapshot.

## Correction event and Undo contract

`correction_events` contains only successfully applied automatic replacements and owns Undo history, not vocabulary validity. Recording a correction does not add, remove, or block a `user_words` row. Undo remains two-phase: external text restoration must succeed first. `commit_correction_undo` then atomically marks the event undone and inserts/updates the restored original in `user_words`, which is the explicit acceptance path after a correction. Persistence therefore cannot claim either correction or Undo success before the corresponding text effect succeeds.

## Runtime snapshot contract

`LanguagePack` owns immutable system exact/delete indexes and `UserLexicon` owns learned exact/delete/prefix indexes. Sequence completion uses an immutable `SequenceHistory` snapshot rebuilt from `text_history`; single-word hiding uses an immutable `CompletionWordSuppressions` snapshot rebuilt from `completion_hidden_words`. `AdaptiveLexicalRuntime::completion_provider` combines lexical, sequence, and suppression snapshots without SQLite access on the completion hot path. The adaptive worker performs DB writes off the keyboard path and swaps rebuilt snapshots only after canonical persistence changes. The Windows replacement probe opens `%LOCALAPPDATA%/SunSwitcher/sunswitcher.db`, so learned history, hidden completion words, and Undo state survive probe restarts.

## High-value anchors

- `src/persistence/database.rs`: bootstrap schema, version gate, settings, system/user vocabulary and correction events.
- `src/persistence/database_tests.rs`: local upsert/Undo invariants.
- `src/persistence/database_certification.rs`: fresh-schema shape/version gate, reopen and snapshot reconstruction.
- `src/lexicon/user_lexicon.rs`: runtime view of `user_words`.
- `src/adaptive/adaptive_runtime.rs`: background mutation/snapshot refresh.
- `src/completion/sequence.rs`: canonical sequence n-grams, context matching, repetition/recency ranking.

## Related memory

- `main/core-project-principles`
- `components/language-dictionaries`
- `components/user-lexicon`
- `components/adaptive-correction`
- `components/clipboard-learning`
- `components/completion`