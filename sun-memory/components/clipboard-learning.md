---
description: Maps typed/copy learning ownership, clipboard observation, self-generated clipboard suppression, and correction-gated admission into user_words; read before changing clipboard observation or automatic vocabulary learning.
---
# Clipboard and Typed Learning

## Purpose

SunSwitcher learns the same local user dictionary from text the user deliberately keeps while typing and from user-owned Unicode clipboard changes. Both sources feed `LearningClient` and canonical SQLite `user_words`; the Windows listener never owns vocabulary policy.

## Typed learning

`AdaptiveCorrectionSession` observes a completed token only when `CorrectionEngine` returns `Keep`. Unknown alphabetic kept words are learned, existing user words advance usage, and system-dictionary words are not duplicated. A token that was automatically replaced is not learned. If that replacement was wrong, successful Pause Undo adds the restored original as an ordinary user word.

## Clipboard learning

`src/windows/clipboard_listener.rs` observes clipboard sequence changes, ignores stale startup contents, retries temporary clipboard lock failures, and forwards stable user-owned Unicode text to `LearningClient::observe_text`. Non-text clipboard states do not teach vocabulary.

Clipboard text is not trusted as already-correct spelling. The adaptive worker runs each extracted token through the same `LexicalCorrectionProvider`, `CorrectionEngine`, and minimum confidence used for typed correction. Only tokens receiving `Keep` are eligible for `user_words`. If a copied token would be replaced (for example a known typo), the erroneous spelling is skipped; the correction target remains available from the system/user dictionary that generated that correction and is the canonical spelling future completion should expose.

## Internal clipboard suppression

Selected-text capture temporarily uses Ctrl+C and then restores the previous clipboard. `InternalClipboardMutationGuard` marks those transport mutations and their final sequence so the watcher does not interpret SunSwitcher-generated clipboard traffic as user learning intent.

## Validation

Listener certification covers startup/no-change, stable user reads, internal mutation suppression and final restore suppression. Adaptive certification covers one kept copied word becoming a typo target and copied tokens that the corrector would replace not entering `user_words`.

## Related memory

- `components/adaptive-correction`
- `components/user-lexicon`
- `components/persistence`
- `main/core-project-principles`