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

Clipboard text is not trusted as already-correct spelling. The adaptive worker runs each extracted token through the same `LexicalCorrectionProvider`, `CorrectionEngine`, and minimum confidence used for typed correction. Only tokens receiving `Keep` are eligible for `user_words`. Accepted words refresh the working lexical snapshot before the next token in the same clipboard payload, so a later typo cannot be learned merely because its target was first learned earlier in that copy. If a copied token would be replaced, the erroneous spelling is skipped for vocabulary and the correction target is used in the canonical sequence observation. Canonical tokens are expanded into bounded contiguous 2-5 word n-grams and upserted into `text_history`; the fact that a token required correction gives no ranking bonus.

## Internal clipboard suppression

Selected-text capture temporarily uses Ctrl+C and then restores the previous clipboard. `InternalClipboardMutationGuard` marks those transport mutations and their final sequence so the watcher does not interpret SunSwitcher-generated clipboard traffic as user learning intent.

## Validation

Listener certification covers startup/no-change, stable user reads, internal mutation suppression and final restore suppression. Adaptive certification covers one kept copied word becoming a typo target, copied tokens that the corrector would replace not entering `user_words`, canonical corrected sequence storage, and repeated sequence observations outranking a one-off corrected observation.

## Related memory

- `components/adaptive-correction`
- `components/user-lexicon`
- `components/persistence`
- `components/completion`
- `main/core-project-principles`