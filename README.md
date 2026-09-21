# SunSwitcher

Windows keyboard correction experiment implemented in Rust.

The current focus is reliable Windows text replacement plus a local SQLite persistence foundation for clipboard history, language data, user vocabulary, correction history, and future completion memory.

## Typed-word replacement probe

```powershell
cargo run --bin replacement_probe
```

Keep the probe running and focus any Windows text field. Focusing/clicking invalidates the tracked caret state intentionally, so press an unambiguous boundary such as Space once before the first test word. Then type an example followed by Space, Enter, or Tab:

- `дял`, `ддля`, `ддляя`, `lkz`, `llkz`, `lzk` -> `для`
- `тчо` -> `что`
- `hlelo`, `helllo`, `руддщ` -> `hello`

Immediately after an automatic correction completed by an ordinary character boundary such as Space or punctuation, press plain `Pause` (`PS` on this keyboard) to Undo it. The default binding is stored in SQLite settings so a later UI can change it. Undo is deliberately immediate-only: any intervening typing, mouse/focus change, or other uncertain input disarms it, and corrections completed with Enter/Tab are not reversed by this hotkey because those keys can have application-specific side effects. Plain Pause is consumed as the SunSwitcher command; modified Pause combinations remain available to Windows/other applications.

The typed-word probe now loads the Russian and English seed dictionaries from SQLite into language-agnostic runtime `LanguagePack` snapshots. The seed lexicons are deliberately small architecture/certification data rather than production-complete dictionaries; their keyboard-layout transforms remain language-layer behavior, while dictionary words/frequencies are canonical database rows. Candidate lookup uses a shared two-deletion in-memory index and weighted Damerau-style scoring: adjacent transpositions are cheap, accidental repeated-key deletions are cheaper than generic edits, and a keyboard-layout transform may be combined with those typo edits in the same candidate path. User vocabulary is persisted separately in `user_terms` and rebuilt as an immutable `UserLexicon`; it preserves canonical spellings such as `QuantileEntryStrategy`, protects exact user terms from speculative correction, supplies typo candidates, and already exposes recency-ranked prefix matches for future autocomplete.

Representative certified examples include `дял`, `ддля`, `ддляя` -> `для`; `hlelo`, `helllo` -> `hello`; and wrong-layout-plus-typo inputs such as `lkz`, `llkz`, `lzk` -> `для` and `руддщ`, `рдудщ`, `рудддщ` -> `hello`. The Windows input contract also keeps the physical identity of layout-ambiguous OEM keys, so wrong-layout Russian words that appear in English layout with punctuation-looking characters are supported: `;bpym` -> `жизнь`, `'[j` -> `эхо`, `j,]trn` -> `объект`, `,scnhj` -> `быстро`, and `k.lb` -> `люди`. Literal punctuation remains literal (`hello,` stays unchanged, while `hlelo,` -> `hello,`), including punctuation that appears only after layout conversion (`руддщб` -> `hello,`, `рдудщб` -> `hello,`). A token that is already an exact dictionary word in any configured language is kept unchanged.

## Selected-text replacement probe

```powershell
cargo run --bin selection_probe
```

This probe validates the separate large-selection path. The clipboard is used only to capture the selected source text: SunSwitcher snapshots a supported pre-existing clipboard state, sends `Ctrl+C`, reads the selection, restores and verifies the original clipboard, and only then changes the selection with direct Unicode `SendInput` events. Replacement text is never transported through the clipboard.

1. Put something recognizable in the clipboard. Plain Unicode text, copied files, and DIB/DIBV5 images such as Windows screenshots are supported by the current preservation contract.
2. In another application, select any text with the mouse or keyboard.
3. Press `F8`.
4. The selected text should become `[SunSwitcher]<selected text>[/SunSwitcher]`.
5. Paste normally afterward. The clipboard content from step 1 should still work. For a copied file, paste it into an Explorer folder; for text, paste into a text field; for a screenshot/image, paste it into an image-capable target.

Copied-file preservation stores the critical `CF_HDROP` payload and `Preferred DropEffect` when present. Image preservation stores every available `CF_DIBV5`/`CF_DIB` payload and verifies the restored bytes before replacement. Other file/image helper or custom clipboard formats are not claimed as preserved. Clipboard states outside the explicit text/file/image contracts fail closed before `Ctrl+C` until they receive their own preservation contract.

Try this in Notepad, Notepad++, browser text fields, Telegram, VS Code, and other editors. If an application has no normal copy-selection semantics, the selected-text path intentionally fails closed instead of guessing.