---
description: Maps SunSwitcher's canonical SQLite ownership, schema-v10 evolution, typed application settings, built-in FST language enablement, user/ignored words, runtime snapshots, clipboard history, and persistence validation; read before changing schema, settings, dictionaries, vocabulary overrides, clipboard storage, or DB-backed runtime state.
---
# SunSwitcher Persistence

## Purpose

One local SQLite database owns mutable persistent application state. Keyboard/correction/completion hot paths consume validated immutable in-memory snapshots and never query SQLite. Shipped RU/EN vocabulary is immutable build data and therefore lives in FST assets rather than mutable database rows.

## Ownership boundary

`src/persistence/database.rs` owns database opening, the canonical bootstrap schema, `PRAGMA user_version`, typed persistence APIs, stored-row validation, language enablement, clipboard history rows, and conversion into runtime snapshots. It does not own lexical correction policy, keyboard transforms, clipboard OS capture, or UI behavior.

The `languages` table owns enabled language IDs. Built-in `ru` and `en` resolve to immutable assets through `builtin_language_pack`; enabled non-built-in language codes fail closed instead of falling back to mutable dictionary rows. Learned vocabulary is separately language-neutral in `user_words`; `ignored_words` is the normalized negative override applied to both built-in and learned vocabulary.

## Schema evolution

The current schema version is 10. Version 1 is the bootstrap baseline, version 2 adds `completion_hidden_words`, version 3 removes legacy grave/backtick pollution, version 4 is retained as a compatibility step, version 5 drops the obsolete `dictionary_words` table after built-in vocabularies moved to immutable FST assets, version 6 adds clipboard ordering indexes separate from clipboard timestamps, version 7 adds persistent `ignored_words`, and version 8 adds typed persistent application settings for hotkeys, feature toggles, and notification timeout. Version 9 adds the suppression-only `application_replacement_suppressions` relation keyed by canonical full executable path; despite its legacy table name, each row now means full SunSwitcher input exclusion for that foreground application. Its creation is idempotent for compatibility replays. Version 10 adds the expression/range index `text_history_retention_idx` on `min(use_count, 12), last_used_at_ms`; the migration itself does not delete evidence. V8 column migration checks each column before adding it so compatibility certifications that replay an older `user_version` over a newer table shape remain idempotent. Existing older databases upgrade in order before use. `PRAGMA user_version` is the compatibility gate; databases newer than the supported schema fail closed.

Schema v5 deliberately removes the custom-language dictionary fallback. Runtime language packs are built only from enabled built-in language IDs plus immutable assets; user vocabulary remains in `user_words`.

## User-word and sequence persistence

`user_words` stores canonical spelling, normalized identity, use count, and last-use time. `ignored_words` stores normalized spellings and is the stronger negative lexical override. `Database::ignore_word` atomically inserts the ignore row and removes any matching `user_words` row; `Database::forget_word` atomically removes the spelling from both stores. Runtime learning rejects ignored spellings, preserving the invariant that one normalized spelling cannot remain both learned and ignored. `load_user_lexicon` and `load_ignored_words` validate the two runtime views. `text_history` remains the canonical persistent sequence authority, subject to inactivity retention owned by `Database::maintain`. `Database::load_active_text_history` exposes only a bounded RAM projection for completion bases, retaining strongest repeated evidence and freshest singleton evidence; projection limits themselves never delete persisted rows. Completion still filters that active view through the effective lexical snapshot. `clipboard_entries` plus typed child tables own clipboard history and pinned state; clipboard Clear/Clear All does not alter lexical or sequence learning state. Clipboard list reads use `ClipboardEntrySummary`, a payload-light projection that carries text or image-format metadata but never image BLOB bytes; `ClipboardEntryView` remains the full payload contract and is loaded by ID only for activation/preview. Summary list APIs return the full ordered history and preserve Unicode case-insensitive filtering semantics without the UI's former 100-row display cap.

Schema v3's grave/backtick cleanup is a one-time migration, not restart behavior. `text_history` owns canonical multi-word sequence evidence, and `completion_hidden_words` independently owns single-word autocomplete suppression. These mutable stores are unaffected by the built-in dictionary asset migration.

## Automatic maintenance and offline compaction

`Database::maintain(now_ms)` atomically expires sequences and prunes clipboard history in one immediate SQLite transaction, returning committed deletion counts in `MaintenanceReport`. A sequence expires only when `last_used_at_ms` is strictly earlier than the UTC cutoff obtained by subtracting `min(use_count, 12)` calendar months from `now_ms`, clamping the day at month-end and preserving milliseconds. An exact cutoff survives. Reobservation naturally renews the last-use timestamp; surviving rows are not rewritten. Twelve indexed retention-class range deletes avoid repeated full-history scans. Unsupported maintenance timestamps fail before deletion.

Clipboard pruning retains the newest unpinned prefix in canonical `current_order_index DESC, id DESC` order until both the existing unpinned count limit and the inclusive 200 MiB unpinned payload budget hold. Payload size is derived directly from persisted UTF-8 text bytes, image BLOB plus UTF-8 format bytes, and all saved file-path UTF-8 bytes. No shadow size counter is stored. Pinned rows are outside both constraints, even when a pinned payload alone exceeds 200 MiB. An oversized newest unpinned row is removable. Parent deletion cascades to typed payload children. Clipboard observation uses this same pruning statement; file rows cannot bypass storage accounting even though the current manager shows text/images only.

Production `SunSwitcherProcessor::new` runs maintenance before runtime snapshot loading and hook registration. One stoppable daily scheduler sends `LearningClient::maintain` to the existing serialized adaptive worker; it does not open another persistence authority. The worker rebases the sequence snapshot after sequence deletion, and a successful clipboard deletion report invalidates the manager’s event-driven summary cache. Failures propagate to the caller and worker error state; startup fails closed and periodic errors are reported by existing runtime diagnostics. The daily interval also applies after failure, preventing tight retry scans. User/ignored words, settings, completion suppressions, correction events, and application suppression state are never maintenance targets.

Runtime maintenance does not compact the file. The explicit offline utility `database_maintenance <existing-db> [--vacuum]` opens without SQLite CREATE, invokes the same owner maintenance, optionally calls `Database::vacuum_exclusive`, then reopens through production persistence. Offline VACUUM obtains and retains exclusive access until the connection closes and verifies its truncating WAL checkpoint. Use a consistent backup and stop the app gracefully before offline work; never replace the database with an empty file. The configured external build directory contains the utility and application binaries.

Owner-local certification is wired from `database.rs` to `database_certification.rs`; the former standalone ordinary database test file is migrated and removed. Retention and count/order Domain families complement exact calendar/millisecond, mixed-byte, pinned, rollback, migration/index, reopen and compaction cases. The persistence/completion composition certification proves base/overlay invalidation through the real adaptive maintenance command.

## Correction event and Undo contract

`correction_events` contains only successfully applied automatic replacements and owns Undo history, not vocabulary validity. Recording a correction does not add/remove/block lexical state. Undo remains two-phase: external text restoration succeeds first, then `commit_correction_undo` atomically marks the event undone, removes a matching `ignored_words` override, and upserts the restored original into `user_words` as explicit re-acceptance.

## Runtime snapshot contract

`Database::load_enabled_language_packs` first reads enabled IDs. Built-in RU/EN IDs are resolved to cached FST-backed `LanguagePack`s; any other enabled ID fails closed because schema v5 removed mutable custom dictionary rows. `AdaptiveLexicalRuntime` combines those packs with `UserLexicon` plus immutable `IgnoredWords`, compact base-plus-overlay sequence state, and completion suppressions. User/ignore membership mutations reload the filtered lexical snapshot off the keyboard hot path; routine sequence writes update only the bounded overlay. `AppSettings` is the typed persistent owner for configurable hotkeys, autocomplete/autocorrection/auto-layout-switch toggles, clipboard history limit, and notification timeout. `HotkeyBinding` persists the canonical Windows virtual-key identity; Ctrl-modified `VK_CANCEL` (the Windows Ctrl+Pause/Break form) is normalized to `VK_PAUSE`, so legacy `ctrl+vk:3` rows load as the same binding and future saves use `ctrl+vk:19`. Production constructs `SettingsStore::from_database` once with a dedicated long-lived writer connection. Both Settings UI and tray use its serialized `update` gateway to derive changes from the latest global value, save the complete canonical settings row, and only then publish the RAM projection. SQLite writes never hold the snapshot lock, so keyboard readers do not wait on the database writer. Keyboard, correction, popup, and clipboard consumers read that shared projection rather than querying SQLite per key event. There is no independent tray boolean or Settings-owned writer. File-backed connections use WAL journal mode, `synchronous=NORMAL`, and a bounded busy timeout to reduce rollback-journal write amplification and tolerate short writer overlap; in-memory databases retain SQLite's memory journal. Keyboard hot paths do no SQLite I/O. Production diagnostics are written to `%LOCALAPPDATA%\SunSwitcher\sunswitcher.log`, beside `sunswitcher.db`, so moving the executable never strands mutable application state or logs next to the binary.

## Application replacement overrides

`Database::load_application_overrides` and `set_application_suppressed` own the canonical collection independently of singleton `AppSettings`. Only explicit full-input suppression is persisted; removing a row restores the unchanged global behavior. `ApplicationIdentity` in `src/application_policy.rs` validates and normalizes full Windows executable paths (drive-absolute or UNC, case-folded, separator-normalized, extended-prefix-normalized), rejecting relative paths, dot components, ambiguous trailing spaces/dots, streams, and malformed identities. Database loads reject noncanonical keys rather than silently merging aliases. Matching is lexical full-path matching, not basename/PID/game detection or filesystem alias resolution.

The shared `SettingsStore` contains an immutable `Arc<ApplicationOverrides>` projection and persists application changes through the same serialized writer before publishing them. Global toggles never mutate application rows. Application suppression never mutates global settings.

## High-value anchors

- `src/persistence/database.rs`: schema/version gate, settings, built-in language enablement, clipboard history, user vocabulary, correction events.
- `src/persistence/database_certification.rs`: schema migration, `dictionary_words` removal, unknown-language fail-closed behavior, reopen behavior.
- `src/language/builtin.rs`: cached RU/EN FST asset loading.
- `src/language/language_pack.rs`: immutable FST/exact/completion and correction-index runtime views.
- `src/lexicon/user_lexicon.rs`: runtime view of `user_words`.
- `src/adaptive/adaptive_runtime.rs`: background mutation/snapshot refresh.
- `src/settings.rs` / `src/settings_certification.rs`: shared global/application projection, serialized persistence-first writer, concurrent writes and failure-path proof.
- `src/application_policy.rs` / `src/application_policy_certification.rs`: canonical executable identity and effective-policy permission family.
- `src/windows/foreground_application.rs`: cached Windows process discovery and recent external-application selection.

## Related memory

- `main/core-project-principles`
- `components/language-dictionaries`
- `components/user-lexicon`
- `components/adaptive-correction`
- `components/clipboard-learning`
- `components/completion`