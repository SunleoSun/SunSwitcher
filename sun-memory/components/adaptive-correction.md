---
description: Maps live lexical snapshot ownership, one correction threshold, background learning, correction-event recording, immediate hotkey Undo, and user-word refresh; read before changing adaptive learning, hot-path snapshot access, correction orchestration, or Undo wiring.
---
# Adaptive Correction Runtime

## Purpose

`src/adaptive/adaptive_runtime.rs` connects input/correction with canonical SQLite state without database I/O in the keyboard hot path. It owns the current immutable lexical snapshot, the background DB worker, the correction confidence threshold, learning commands, and adaptive correction sessions.

## Ownership boundary

`AdaptiveLexicalRuntime::start` builds the initial `LexicalSnapshot` from enabled system `LanguagePack`s plus `UserLexicon`. The runtime owns one minimum correction confidence and gives that same value to typed sessions and clipboard-text learning so admission cannot disagree with automatic correction policy.

The background worker is the adaptive runtime component that mutates `Database`. Successful user-word mutations reload `UserLexicon` from canonical `user_words` rows and atomically swap the derived snapshot while reusing unchanged language packs.

## Learning contract

Typed learning occurs only for completed tokens whose correction decision is `Keep`. Unknown alphabetic kept tokens are added to `user_words`; exact system words are not duplicated. Existing user words are re-observed to advance usage/recency.

`LearningClient::observe_text` tokenizes trusted copied text and runs every token through `LexicalCorrectionProvider` + `CorrectionEngine` at the same runtime confidence threshold. Only `Keep` tokens may be admitted. A copied source spelling that would be corrected is therefore never taught as valid vocabulary.

## Hot-path contract

Correction lookup uses immutable snapshots only. A proposed replacement remains pending until the Windows side-effect owner reports `ReplacementOutcome::Applied`; only then is the correction event persisted. Intervening input, downstream suppression, stale foreground/revision ownership, or failed injection aborts the pending effect. The keyboard hook never waits on SQLite.

## Undo contract

Automatic replacements are recorded in `correction_events` only after they actually apply. Undo is two-phase: restore text externally first, then commit persistence. A successful commit atomically marks the exact correction event undone and inserts/updates the restored original in `user_words`; no separate Protected state exists. Immediate Undo remains limited to safely reversible character-boundary corrections and is disarmed by intervening input.

## High-value anchors

- `src/adaptive/adaptive_runtime.rs`: snapshots, threshold, learning worker, session, correction receipt/Undo flow.
- `src/adaptive/adaptive_runtime_tests.rs`: admission invariants.
- `src/adaptive/adaptive_runtime_certification.rs`: typed/copy learning, live correction, effect persistence and Undo.
- `src/persistence/database.rs`: canonical user-word and correction-event operations.
- `src/windows/keyboard_runtime.rs`: actual effect ownership/injection result.

## Validation

Run adaptive tests/certification, persistence and lexical-provider certifications, then full tests, bin check, Clippy with warnings denied, formatting and diff checks.

## Related memory

- `components/persistence`
- `components/user-lexicon`
- `components/clipboard-learning`
- `main/core-project-principles`