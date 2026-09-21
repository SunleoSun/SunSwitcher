---
description: Maps live lexical snapshot ownership, background learning, automatic technical-term admission, correction-event recording, immediate hotkey Undo, and protected Undo refresh; read before changing adaptive learning, hot-path snapshot access, correction orchestration, or Undo wiring.
---
# Adaptive Correction Runtime

## Purpose

`src/adaptive/adaptive_runtime.rs` connects input/correction with canonical SQLite state without moving database I/O into the keyboard hot path. It owns the current immutable lexical snapshot, the background database worker, typed learning commands, and the reusable adaptive correction session.

## Ownership boundary

`AdaptiveLexicalRuntime` builds the initial `LexicalSnapshot` from enabled `LanguagePack` rows plus `UserLexicon`, then gives hot-path consumers a `LexicalSnapshotStore`. `AdaptiveCorrectionSession` owns token completion orchestration: it reads one current snapshot, makes a lexical decision, queues learning or correction-history persistence, and produces a typed replacement directive. It does not own Windows injection or SQLite schema semantics.

The background learning worker is the only adaptive runtime component that mutates `Database`. Successful user-term mutations rebuild `UserLexicon` from canonical rows and swap a new snapshot while reusing the unchanged language packs.

## Learning contract

A kept token already present in `UserLexicon` is re-observed so usage count and recency can advance. Exact system-dictionary words are not duplicated into user vocabulary. A new unknown token is automatically admitted only when it has a technical-identifier shape such as mixed case, digits, underscore/hyphen, or an acronym-like uppercase form; a single ordinary unknown lowercase word is not promoted into an exact known word after one observation. This avoids teaching a one-off typo as valid vocabulary while still learning identifiers such as `QuantileEntryStrategy` and `RDNA2`.

`LearningClient::observe_text` applies the same admission policy to tokenized text and is the reusable future entrypoint for clipboard text. Numeric/symbol-only fragments are not user terms.

## Hot-path contract

The keyboard/correction path never calls SQLite. Snapshot reads use the in-memory store; persistence commands are sent to the background worker. A proposed correction is held as pending until the Windows side-effect layer reports `ReplacementOutcome::Applied`; only then is correction history queued. Any intervening observed input cancels that pending ownership, and the Windows runtime also checks an input revision across downstream hooks and synthetic injection so a re-entrant caret/text change cannot execute or persist a stale replacement. A downstream suppression, stale revision, or failed injection reports `Aborted` and discards the pending history. The correction-history command returns a private receipt rather than making the hook wait for SQLite. If immediate Undo succeeds, the same worker later consumes that receipt in FIFO order and commits the exact recorded event. Background persistence failures leave the prior snapshot authoritative and are surfaced by the learning client barrier/error path.

## Undo contract

Automatic replacements are queued into canonical `correction_events`. Undo remains two-phase: text restoration must succeed before persistence may mark the event undone and protect the restored original. The immediate typed-word path arms a reverse action only for a correction ending in an ordinary character boundary; Enter/Tab fail closed because replaying their application-specific effects is not safely reversible. Any intervening input disarms the candidate. The Windows runtime resolves the hotkey only after downstream hooks return, so re-entrant input can invalidate the candidate before execution. `UndoOutcome::Applied` queues the background commit/protection, `NotExecuted` is retryable only when injection sent no input, and `Uncertain` discards the candidate and invalidates tracking after a partial/unknown side effect. The existing prepare/commit API remains the explicit two-phase contract for future non-immediate UI Undo flows.

## High-value navigation anchors

- `src/adaptive/adaptive_runtime.rs`: snapshot store, worker, learning policy, adaptive correction session, Undo refresh.
- `src/adaptive/adaptive_runtime_tests.rs`: local learning/admission invariants.
- `src/adaptive/adaptive_runtime_certification.rs`: live snapshot, typo-correction, recency, immediate Undo, and Undo-refresh behavior.
- `src/persistence/database.rs`: canonical user-term and correction-event operations.
- `src/correction/lexical_provider.rs`: immutable `LexicalSnapshot` and lexical candidate policy.
- `src/bin/replacement_probe.rs`: Windows probe consuming the adaptive session.

## Validation

Run adaptive unit tests/certification first, then persistence and lexical-provider certifications because changes here span snapshot refresh, DB mutation, and correction behavior. Finish with the full Rust tests, bin checks, Clippy with warnings denied, formatting, and diff checks.

## Revisit when

Revisit this entry when automatic admission policy, snapshot ownership/swap behavior, worker lifecycle/error propagation, correction-event recording, Undo ordering, clipboard learning, or the adaptive session contract changes.

## Related memory

- `components/persistence`
- `components/user-lexicon`
- `main/core-project-principles`

## Source inspection still required

Inspect the current worker command handling, admission predicate, correction-event receipt flow, and persistence Undo implementation before changing behavior. Immediate typed-word restoration is already wired through the Windows keyboard runtime; a future settings UI must mutate canonical `AppSettings` rather than creating another hotkey authority.