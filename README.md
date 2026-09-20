# SunSwitcher

Windows keyboard correction experiment implemented in Rust.

The current focus is proving reliable Windows text replacement before adding language/layout switching, dictionaries, persistence, or UI.

## Typed-word replacement probe

```powershell
cargo run --bin replacement_probe
```

Keep the probe running and focus any Windows text field. Focusing/clicking invalidates the tracked caret state intentionally, so press an unambiguous boundary such as Space once before the first test word. Then type an example followed by Space, Enter, or Tab:

- `дял`, `ддля`, `ддляя`, `lkz`, `llkz`, `lzk` -> `для`
- `тчо` -> `что`
- `hlelo`, `helllo`, `руддщ` -> `hello`

The typed-word probe now uses the language-agnostic lexical provider with separate Russian and English language packs. The current built-in dictionaries are deliberately small seed lexicons for architecture/certification rather than production-complete dictionaries. Candidate lookup uses a two-deletion index and weighted Damerau-style scoring: adjacent transpositions are cheap, accidental repeated-key deletions are cheaper than generic edits, and a keyboard-layout transform may be combined with those typo edits in the same candidate path.

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