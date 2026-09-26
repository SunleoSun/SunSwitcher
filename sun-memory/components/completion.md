---
description: Maps SunSwitcher's word and multi-word completion owners, canonical sequence history, context specificity, user sequence score, and immutable runtime snapshots; read before changing autocomplete ranking, text_history, or completion lookup.
---
# Completion

## Ownership

`src/completion/` owns completion lookup and ranking. `CompletionProvider` consumes immutable runtime state only: the current lexical snapshot for system/user word completion, the current `SequenceHistory` snapshot for multi-word completion, and the current completion-word suppression snapshot. Completion lookup never queries SQLite. `SequenceHistory` precomputes tokenized and normalized forms when its immutable snapshot is built, so the keyboard hot path does not repeatedly split and normalize every persisted `text_history` row on each keystroke.

## Canonical sequence history

Clipboard text observation reuses the same lexical correction decision used for vocabulary admission. Each observed token contributes its kept spelling or the confident correction replacement to the canonical token stream. Raw spellings that the corrector would replace are therefore not prediction targets. The canonical stream is expanded into contiguous 2-5 word n-grams, persisted as whole canonical strings in `text_history`, and rebuilt into `SequenceHistory`. `text_history.use_count` increases only because that canonical n-gram is observed again; correction itself adds no bonus. Physical typing also feeds the same `text_history`: `AdaptiveCorrectionSession` owns a rolling context of the last five proven typed words and emits only suffix n-grams ending at each newly resolved word, so older n-grams are not counted again on every keystroke. Completion receives the same resolved canonical word from the correction session, so a kept or corrected token whose physical layout key deferred punctuation (for example `hello,`) contributes `hello` to completion context rather than a second raw interpretation. Punctuation-only tokens clear context. An applied automatic correction enters the rolling context as the canonical corrected word but its sequence observation is deferred while immediate Undo is still possible; a successful Undo replaces that last rolling word with the canonical restored original before the sequence is persisted. Aborted or uncertain replacement effects contribute no sequence evidence.

## Multi-word ranking

Context specificity is a separate, higher-priority ranking dimension, not part of the numeric user score. Up to the last four completed context words may match a stored sequence prefix. Sequence lookup also applies the current partial-word prefix before truncating candidates, so a phrase can be suggested directly from its first word even when there is no completed context. Candidates sort by matched context-word count descending, then by user score descending; equal evidence prefers the longer remaining continuation, then deterministic text order. Sequence lookup has one canonical prefix-driven ranking path. With a visible prefix it may deliberately use a less-specific repeated context when the most-specific match would collapse a learned sequence to one displayed word; it prefers the most-specific match that still exposes at least a two-word continuation, falling back to the ordinary most-specific match when none exists. Without a prefix, ordinary context specificity wins. Alt+Right does not select another provider or ranking mode: while the physical prefix is temporarily empty after accepting a word, `CompletionSession` feeds the first already-visible protected continuation word back as the logical lookup prefix. Candidate generation/ranking is therefore the same code path used for manual typing; only session lifecycle owns the source of the prefix. This keeps evidence such as repeated `AA33 AA33` visible without a second completion behavior.

For an entry with `repeat_count = use_count - 1`, the current user score is:

`4 * ln(1 + repeat_count) + exp(-age_days / 14)`

Repetition is intentionally the dominant evidence; recency is a bounded secondary signal. A first observation has no repetition component.

## Word completion

Word completion merges learned `UserLexicon` prefix matches with system `LanguagePack` prefix matches. Prefix lookup and deduplication are normalized/case-insensitive. The current word policy keeps learned user words ahead of system rows; learned words preserve their existing recency/use-count ordering and system rows preserve dictionary-frequency ordering. Suggestion rendering preserves the exact prefix casing already typed by the user. Uniform lower/upper/title words adapt the untyped remainder to that observed case, while mixed/camel identifiers retain their canonical internal casing after the typed prefix. A separate immutable `CompletionWordSuppressions` snapshot filters words that the user removed from autocomplete with Delete. This is intentionally completion-only: hiding `hello` does not delete the system dictionary row or a learned `user_words` row, so spell/correction authority remains canonical and independent from UI preference. The provider widens the pre-filter lookup by the suppression count before final truncation so hidden words do not unnecessarily reduce visible result count.

## Runtime refresh and live session

`AdaptiveLexicalRuntime` owns the background persistence path. It rebuilds sequence and completion-suppression snapshots after their canonical DB mutations and exposes `completion_provider()` as the typed RAM-only lookup boundary. User-word changes remain owned by the lexical snapshot store; the provider combines lexical, sequence, and suppression snapshots without SQLite access on keystrokes. Sequence completion is a derived view and may surface only words that are still valid in the current lexical snapshot, so deleting a custom `user_words` term immediately removes its authority once the snapshot refreshes even if historical `text_history` rows containing that term remain stored. Pause-based user-word deletion also invalidates the live completion session and closes the current popup so an already-rendered stale row is not left visible while the background snapshot catches up. Core `CompletionSession` owns the current prefix, up to four completed context words, suggestions, selected index, three-character normal activation threshold, whole-suffix acceptance, one-word continuation acceptance, and the typed deletion target for each row. Delete removes the selected row immediately and records the target locally so an asynchronous snapshot refresh cannot make it flash back into the same session. A multi-word suggestion carries the exact originating `text_history` string even when the displayed completion is context-truncated, so deletion targets the canonical stored row. A single-word suggestion carries normalized `Word(...)` identity and persists completion-only suppression. `AdaptiveCompletionSession` also keeps the lexical words represented by an in-flight Accept/AcceptNextWord command until the Windows effect result arrives; only `Applied` acceptance re-observes those words through normal typed-learning semantics, while `Uncertain` discards the pending evidence. This makes accepted learned-word completions advance their user use-count/recency instead of leaving ranking unchanged. After a proven one-word insertion, the not-yet-accepted remainder that the user already saw becomes a protected prefix of the refreshed top suggestion: ranking may only append after it, so repeated Alt+Right never inserts an unseen replacement word. Correction replacements may canonicalize the most recently completed completion-context word after the replacement side effect is confirmed applied, and successful Undo restores that word to the original spelling.

## Validation anchors

- `src/completion/sequence.rs`: context specificity, prefix-filtered sequence lookup, repetition/recency score, longer-continuation tie break, bounded n-gram generation.
- `src/completion/session.rs`: three-character normal trigger, continuation mode, typed context, candidate selection, whole-suffix accept, one-word accept, and protected-prefix extension.
- `src/adaptive/adaptive_runtime_certification.rs`: canonical copied sequence, no correction bonus, repeated sequence wins, first-word phrase prefix completion, stable one-word continuation acceptance, system+user word completion, learned-sequence deletion, system-word completion suppression without lexical deletion, live activation/context completion, Undo-learned completion.
- `src/persistence/database_certification.rs`: text-history repetition survives reopen.

## Related memory

- `components/persistence`
- `components/clipboard-learning`
- `components/user-lexicon`
- `components/autocomplete-ui`
- `main/core-project-principles`
