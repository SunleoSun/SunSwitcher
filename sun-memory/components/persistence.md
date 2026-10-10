---
description: Maps SunSwitcher's canonical SQLite ownership, schema-v8 evolution, typed application settings, built-in FST language enablement, user/ignored words, runtime snapshots, clipboard history, and persistence validation; read before changing schema, settings, dictionaries, vocabulary overrides, clipboard storage, or DB-backed runtime state.
---
# SunSwitcher Persistence

## Purpose

One local SQLite database owns mutable persistent application state. Keyboard/correction/completion hot paths consume validated immutable in-memory snapshots and never query SQLite. Shipped RU/EN vocabulary is immutable build data and therefore lives in FST assets rather than mutable database rows.

## Ownership boundary

`src/persistence/database.rs` owns database opening, the canonical bootstrap schema, `PRAGMA user_version`, typed persistence APIs, stored-row validation, language enablement, clipboard history rows, and conversion into runtime snapshots. It does not own lexical correction policy, keyboard transforms, clipboard OS capture, or UI behavior.

The `languages` table owns enabled language IDs. Built-in `ru` and `en` resolve to immutable assets through `builtin_language_pack`; enabled non-built-in language codes fail closed instead of falling back to mutable dictionary rows. Learned vocabulary is separately language-neutral in `user_words`; `ignored_words` is the normalized negative override applied to both built-in and learned vocabulary.

## Schema evolution

The current schema version is 8. Version 1 is the bootstrap baseline, version 2 adds `completion_hidden_words`, version 3 removes legacy grave/backtick pollution, version 4 is retained as a compatibility step, version 5 drops the obsolete `dictionary_words` table after built-in vocabularies moved to immutable FST assets, version 6 adds clipboard ordering indexes separate from clipboard timestamps, version 7 adds persistent `ignored_words`, and version 8 adds typed persistent application settings for hotkeys, feature toggles, and notification timeout. V8 column migration checks each column before adding it so compatibility certifications that replay an older `user_version` over a newer table shape remain idempotent. Existing older databases upgrade in order before use. `PRAGMA user_version` is the compatibility gate; databases newer than the supported schema fail closed.

Schema v5 deliberately removes the custom-language dictionary fallback. Runtime language packs are built only from enabled built-in language IDs plus immutable assets; user vocabulary remains in `user_words`.

## User-word and sequence persistence

`user_words` stores canonical spelling, normalized identity, use count, and last-use time. `ignored_words` stores normalized spellings and is the stronger negative lexical override. `Database::ignore_word` atomically inserts the ignore row and removes any matching `user_words` row; `Database::forget_word` atomically removes the spelling from both stores. Runtime learning rejects ignored spellings, preserving the invariant that one normalized spelling cannot remain both learned and ignored. `load_user_lexicon` and `load_ignored_words` validate the two runtime views. `text_history` remains the complete persistent historical sequence authority; `Database::load_active_text_history` exposes only a bounded RAM projection for completion bases, retaining strongest repeated evidence and freshest singleton evidence without deleting cold rows. Completion still filters that active view through the effective lexical snapshot. `clipboard_entries` plus typed child tables own clipboard history and pinned state; clipboard Clear/Clear All does not alter lexical or sequence learning state.

Schema v3's grave/backtick cleanup is a one-time migration, not restart behavior. `text_history` owns canonical multi-word sequence evidence, and `completion_hidden_words` independently owns single-word autocomplete suppression. These mutable stores are unaffected by the built-in dictionary asset migration.

## Correction event and Undo contract

`correction_events` contains only successfully applied automatic replacements and owns Undo history, not vocabulary validity. Recording a correction does not add/remove/block lexical state. Undo remains two-phase: external text restoration succeeds first, then `commit_correction_undo` atomically marks the event undone, removes a matching `ignored_words` override, and upserts the restored original into `user_words` as explicit re-acceptance.

## Runtime snapshot contract

`Database::load_enabled_language_packs` first reads enabled IDs. Built-in RU/EN IDs are resolved to cached FST-backed `LanguagePack`s; any other enabled ID fails closed because schema v5 removed mutable custom dictionary rows. `AdaptiveLexicalRuntime` combines those packs with `UserLexicon` plus immutable `IgnoredWords`, compact base-plus-overlay sequence state, and completion suppressions. User/ignore membership mutations reload the filtered lexical snapshot off the keyboard hot path; routine sequence writes update only the bounded overlay. `AppSettings` is the typed persistent owner for configurable hotkeys, autocomplete/autocorrection/auto-layout-switch toggles, clipboard history limit, and notification timeout. `HotkeyBinding` persists the canonical Windows virtual-key identity; Ctrl-modified `VK_CANCEL` (the Windows Ctrl+Pause/Break form) is normalized to `VK_PAUSE`, so legacy `ctrl+vk:3` rows load as the same binding and future saves use `ctrl+vk:19`. The UI persists a complete settings value first, then atomically replaces the shared in-memory `SettingsStore`; keyboard, correction, popup, and clipboard consumers read that live snapshot instead of querying SQLite per key event. File-backed connections use WAL journal mode, `synchronous=NORMAL`, and a bounded busy timeout to reduce rollback-journal write amplification and tolerate short writer overlap; in-memory databases retain SQLite's memory journal. Keyboard hot paths do no SQLite I/O. Production diagnostics are written to `%LOCALAPPDATA%\SunSwitcher\sunswitcher.log`, beside `sunswitcher.db`, so moving the executable never strands mutable application state or logs next to the binary.

## High-value anchors

- `src/persistence/database.rs`: schema/version gate, settings, built-in language enablement, clipboard history, user vocabulary, correction events.
- `src/persistence/database_certification.rs`: schema migration, `dictionary_words` removal, unknown-language fail-closed behavior, reopen behavior.
- `src/language/builtin.rs`: cached RU/EN FST asset loading.
- `src/language/language_pack.rs`: immutable FST/exact/completion and correction-index runtime views.
- `src/lexicon/user_lexicon.rs`: runtime view of `user_words`.
- `src/adaptive/adaptive_runtime.rs`: background mutation/snapshot refresh.
- `src/settings.rs`: shared live projection of typed `AppSettings` for runtime/UI consumers.

## Related memory

- `main/core-project-principles`
- `components/language-dictionaries`
- `components/user-lexicon`
- `components/adaptive-correction`
- `components/clipboard-learning`
- `components/completion`