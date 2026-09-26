---
description: Maps SunSwitcher's canonical SQLite ownership, schema-v5 evolution, built-in FST language enablement, user words, runtime snapshots, clipboard history, and persistence validation; read before changing schema, settings, dictionaries, user words, clipboard storage, or DB-backed runtime state.
---
# SunSwitcher Persistence

## Purpose

One local SQLite database owns mutable persistent application state. Keyboard/correction/completion hot paths consume validated immutable in-memory snapshots and never query SQLite. Shipped RU/EN vocabulary is immutable build data and therefore lives in FST assets rather than mutable database rows.

## Ownership boundary

`src/persistence/database.rs` owns database opening, the canonical bootstrap schema, `PRAGMA user_version`, typed persistence APIs, stored-row validation, language enablement, clipboard history rows, and conversion into runtime snapshots. It does not own lexical correction policy, keyboard transforms, clipboard OS capture, or UI behavior.

The `languages` table owns enabled language IDs. Built-in `ru` and `en` resolve to immutable assets through `builtin_language_pack`; enabled non-built-in language codes fail closed instead of falling back to mutable dictionary rows. Learned vocabulary is separately language-neutral in `user_words`.

## Schema evolution

The current schema version is 5. Version 1 is the bootstrap baseline, version 2 adds `completion_hidden_words`, version 3 removes legacy grave/backtick pollution, version 4 is retained as a compatibility step, and version 5 drops the obsolete `dictionary_words` table after built-in vocabularies moved to immutable FST assets. Existing older databases upgrade in order before use. `PRAGMA user_version` is the compatibility gate; databases newer than the supported schema fail closed.

Schema v5 deliberately removes the custom-language dictionary fallback. Runtime language packs are built only from enabled built-in language IDs plus immutable assets; user vocabulary remains in `user_words`.

## User-word and sequence persistence

`user_words` stores canonical spelling, normalized identity, use count, and last-use time; there is no protection flag. `Database::record_user_word` owns normalized upsert semantics, `delete_user_word` is the explicit normalized deletion path used by Pause on a selection, and `load_user_lexicon` validates rows before constructing the runtime snapshot. Runtime startup loads persisted `user_words` as-is and never reconciles them against correction history. `clipboard_entries` plus typed child tables own clipboard history and pinned state; text clipboard history is deduplicated by kind/content hash, ordered by `last_seen_at_ms` for Current and `pinned_at_ms` for Pinned, and pruned without removing pinned entries.

Schema v3's grave/backtick cleanup is a one-time migration, not restart behavior. `text_history` owns canonical multi-word sequence evidence, and `completion_hidden_words` independently owns single-word autocomplete suppression. These mutable stores are unaffected by the built-in dictionary asset migration.

## Correction event and Undo contract

`correction_events` contains only successfully applied automatic replacements and owns Undo history, not vocabulary validity. Recording a correction does not add/remove/block `user_words`. Undo remains two-phase: external text restoration succeeds first, then `commit_correction_undo` atomically marks the event undone and upserts the restored original into `user_words`.

## Runtime snapshot contract

`Database::load_enabled_language_packs` first reads enabled IDs. Built-in RU/EN IDs are resolved to cached FST-backed `LanguagePack`s; any other enabled ID fails closed because schema v5 removed mutable custom dictionary rows. `AdaptiveLexicalRuntime` then combines those system packs with the `UserLexicon`, sequence-history snapshot, and completion suppressions. Background writes rebuild/swap only the affected mutable snapshots; keyboard hot paths do no SQLite I/O.

## High-value anchors

- `src/persistence/database.rs`: schema/version gate, settings, built-in language enablement, clipboard history, user vocabulary, correction events.
- `src/persistence/database_certification.rs`: schema migration, `dictionary_words` removal, unknown-language fail-closed behavior, reopen behavior.
- `src/language/builtin.rs`: cached RU/EN FST asset loading.
- `src/language/language_pack.rs`: immutable FST/exact/completion and correction-index runtime views.
- `src/lexicon/user_lexicon.rs`: runtime view of `user_words`.
- `src/adaptive/adaptive_runtime.rs`: background mutation/snapshot refresh.

## Related memory

- `main/core-project-principles`
- `components/language-dictionaries`
- `components/user-lexicon`
- `components/adaptive-correction`
- `components/clipboard-learning`
- `components/completion`