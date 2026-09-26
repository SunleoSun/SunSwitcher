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

Typed learning occurs only for completed tokens whose correction decision is `Keep`. Unknown alphabetic kept tokens are added to `user_words`; exact system words are not duplicated. Existing user words are re-observed to advance usage/recency. Successfully applied autocomplete acceptance uses the same typed-token learning command for the lexical words the user chose, so an accepted existing user word advances usage/recency; uncertain or failed insertion does not. Correction history is not a negative vocabulary authority and runtime startup does not reinterpret or clean `user_words`; it simply loads the persisted vocabulary. Learning uses the lexical provider's canonical core rather than the raw deferred token: layout-ambiguous punctuation can be retained temporarily for wrong-layout interpretation but must never become part of a kept user word or rolling sequence, and punctuation-only physical tokens resolve to no lexical word and clear rolling context. The correction session also owns a five-word rolling canonical context for typed multi-word learning. Kept words are appended immediately; applied automatic corrections append the canonical corrected word, stripping rendered punctuation before sequence persistence, but defer that sequence observation while immediate Undo is possible. If Undo succeeds, the canonical restored original replaces the corrected word in the rolling context and is the version written to sequence history. An aborted/uncertain replacement records no sequence evidence and clears the rolling context because visible text is not proven. Intervening input finalizes a non-undone proven correction.

`LearningClient::observe_text` tokenizes trusted copied text and runs every token through `LexicalCorrectionProvider` + `CorrectionEngine` at the same runtime confidence threshold. Only `Keep` tokens may be admitted. A copied source spelling that would be corrected is therefore never taught as valid vocabulary.

## Hot-path contract

Correction lookup uses immutable snapshots only. Physical-layout transformation preserves unmapped non-alphabetic characters such as digits while still failing closed on unmapped letters, so learned identifiers such as `AA33` can be reached from the opposite-layout spelling `ФФ33` without assigning a language to the user word. A one-character spelling that exists only because of the immutable system dictionary may still resolve to an exact one-character word under the opposite physical layout; this prevents system letter/abbreviation rows from blocking legitimate short wrong-layout words such as `z` -> `я` or `b` -> `и`. An explicit learned one-character `user_words` row normally remains authoritative and blocks that reinterpretation, but a learned one-character alias can still yield an opposite-layout short Russian function word (`а`, `в`, `и`, `к`, `о`, `с`, `у`) when that target is itself a stronger learned user word. This keeps accidentally learned aliases such as `b` from blocking frequent `и`, while a heavily used source letter still wins. A proposed replacement remains pending until the Windows side-effect owner reports `ReplacementOutcome::Applied`; only then is the correction event persisted. A wrong-layout candidate now carries its typed target `LanguageId` through `CorrectionReplacement` and `ReplacementAction`; after and only after the replacement is proven applied, the Windows side-effect owner requests that same foreground keyboard layout so subsequent physical typing continues in the corrected language. Same-layout typo corrections carry no target language and never change the OS layout. Intervening input, downstream suppression, stale foreground/revision ownership, or failed injection aborts the pending effect. The keyboard hook never waits on SQLite.

## Undo contract

Automatic replacements are recorded in `correction_events` only after they actually apply. Undo is two-phase: restore text externally first, then commit persistence. A successful commit atomically marks the exact correction event undone and inserts/updates the restored original in `user_words`; no separate Protected state exists. Pause has two explicit meanings: with no current selection it requests this immediate Undo, while with an existing text selection it does not Undo and instead queues deletion of that normalized spelling from `user_words`. The selected-word path immediately invalidates correction/completion tracking and dismisses the current popup, while the background worker performs persistence and then reloads/swaps the live `UserLexicon` snapshot off the keyboard hot path. Immediate Undo remains limited to safely reversible character-boundary corrections and is disarmed by intervening input.

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