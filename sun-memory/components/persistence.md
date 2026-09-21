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

There are no released/existing SunSwitcher databases that require compatibility yet, so historical development migrations were flattened into one current bootstrap schema at version 1. `PRAGMA user_version` remains the future compatibility mechanism, and a database newer than the supported schema still fails closed. Once a database version must be preserved for real users, subsequent schema changes must become forward migrations instead of rewriting the released baseline.

Russian and English system surface forms/frequencies are canonical DB rows; valid inflections are exact dictionary entries rather than suffix heuristics. Language keyboard transforms remain in `src/language/`. `AppSettings` owns the clipboard history limit and Undo hotkey.

## User-word persistence

`Database::record_user_word` owns learned-word upsert semantics: normalized identity is unique, use count accumulates, last-use time never moves backward, and the first accepted canonical spelling remains stable across later case variants. `Database::load_user_lexicon` validates normalized spelling and typed counters before constructing the runtime snapshot.

## Correction event and Undo contract

`correction_events` contains only successfully applied automatic replacements. Undo remains two-phase: external text restoration must succeed first. `commit_correction_undo` then atomically marks the event undone and inserts/updates the restored original in `user_words`. Persistence therefore cannot claim either correction or Undo success before the corresponding text effect succeeds.

## Runtime snapshot contract

`LanguagePack` owns immutable system exact/delete indexes and `UserLexicon` owns learned exact/delete/prefix indexes. `AdaptiveLexicalRuntime` performs DB writes off the keyboard path and swaps a rebuilt user snapshot only after canonical persistence changes.

## High-value anchors

- `src/persistence/database.rs`: bootstrap schema, version gate, settings, system/user vocabulary and correction events.
- `src/persistence/database_tests.rs`: local upsert/Undo invariants.
- `src/persistence/database_certification.rs`: fresh-schema shape/version gate, reopen and snapshot reconstruction.
- `src/lexicon/user_lexicon.rs`: runtime view of `user_words`.
- `src/adaptive/adaptive_runtime.rs`: background mutation/snapshot refresh.

## Related memory

- `main/core-project-principles`
- `components/language-dictionaries`
- `components/user-lexicon`
- `components/adaptive-correction`
- `components/clipboard-learning`