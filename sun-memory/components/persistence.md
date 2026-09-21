---
description: Maps SunSwitcher's canonical SQLite ownership, schema evolution, runtime snapshot loading, and persistence validation; read before changing migrations, settings, dictionaries, user terms, clipboard storage, or DB-backed runtime state.
---
# SunSwitcher Persistence

## Purpose

SunSwitcher uses one local SQLite database as the canonical owner of persistent application state. Runtime and UI state are derived from it; latency-sensitive keyboard, correction, and completion paths must use validated in-memory snapshots rather than querying SQLite.

## Ownership boundary

`src/persistence/database.rs` owns database opening, `PRAGMA user_version` migrations, typed persistence APIs, stored-row validation, and conversion from canonical rows into runtime snapshots. It does not own lexical correction policy, language keyboard transforms, clipboard OS capture, or UI behavior.

The database currently persists application settings, enabled languages and dictionary words, user terms, correction history, text history, and typed clipboard history payloads. Avoid adding tables or columns until a concrete behavior or invariant requires them.

## Schema evolution

Schema evolution is sequential and transactional through SQLite `PRAGMA user_version`. Existing migration steps are append-only compatibility contracts: change persisted semantics with a new schema version instead of rewriting an older migration that an existing database may already have applied. There is intentionally no separate migration-history table. A database newer than the binary's supported schema fails closed instead of being opened with guessed compatibility.

Seed Russian and English dictionary words and frequencies are canonical database rows. Language-specific keyboard-layout transforms remain owned by `src/language/`; `Database::load_enabled_language_packs` validates rows and then asks the language layer to build immutable `LanguagePack` snapshots. `AppSettings` also owns the typed Undo hotkey setting; the current default is plain Pause, and unknown stored bindings fail closed. Schema migration supplies that default for existing databases so later UI work can edit the same canonical setting.

## User-term persistence

`user_terms` is the canonical store for the local user vocabulary. `Database::record_user_term` owns upsert semantics: normalized identity is unique, use count accumulates, last-use time never moves backward, explicit protection is sticky, and a later unprotected observation cannot overwrite the canonical spelling of an already protected term. `Database::load_user_lexicon` validates stored normalization and typed fields before building the runtime `UserLexicon`.

## Correction event and Undo contract

`correction_events` is the canonical history of successfully applied automatic replacements. The adaptive session keeps a proposed replacement pending until the Windows side-effect layer reports that injection actually succeeded; aborted or suppressed replacements are not persisted as correction events. Undo is deliberately two-phase: text restoration happens first, and only a confirmed successful restoration may `commit_correction_undo` mark the event undone and protect the original in `user_terms` atomically. Immediate hotkey Undo carries the exact correction-event receipt through the background worker rather than querying SQLite from the hook; explicit future UI Undo can use `prepare_correction_undo` followed by the same commit rule. This prevents persistence from claiming either a correction or an Undo succeeded when the corresponding text mutation did not happen.

## Runtime snapshot contract

SQLite rows are not a hot-path search structure. `LanguagePack` owns its in-memory exact/delete indexes and `UserLexicon` owns its exact/delete/prefix-searchable runtime view. `AdaptiveLexicalRuntime` moves database writes to a background owner and swaps a rebuilt user-lexicon snapshot only after successful persistence. Snapshots must be reconstructable from canonical database rows and consumers must not maintain a second persistent authority.

## High-value navigation anchors

- `src/persistence/database.rs`: schema, migrations, settings, dictionary loading, and user-term persistence.
- `src/persistence/database_tests.rs`: local persistence invariants.
- `src/persistence/database_certification.rs`: persisted behavior across reopen, migration/version handling, and runtime snapshot reconstruction.
- `src/language/`: language-specific transform construction consumed after DB loading.
- `src/lexicon/user_lexicon.rs`: runtime user-vocabulary snapshot built from `user_terms`.
- `src/adaptive/adaptive_runtime.rs`: background DB mutation and live snapshot refresh outside the keyboard hot path.

## Validation

Use the colocated persistence unit tests for typed update semantics and database certification tests for reopen/runtime-rebuild behavior. Persistence changes should also run the relevant consumer certifications because a schema/load change can preserve SQL correctness while changing correction behavior.

## Revisit when

Revisit this entry when schema ownership, migration behavior, persistence APIs, database-to-snapshot conversion, or the canonical-vs-runtime boundary changes.

## Related memory

- `main/core-project-principles`
- `components/user-lexicon`
- `components/adaptive-correction`

## Source inspection still required

Inspect current migration SQL and typed loader/update implementations before editing persisted contracts; memory intentionally does not duplicate exact SQL or schema-version numbers.