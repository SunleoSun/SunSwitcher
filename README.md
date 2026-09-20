# SunSwitcher

Windows keyboard correction experiment implemented in Rust.

The current focus is proving reliable Windows text replacement before adding language/layout switching, dictionaries, persistence, or UI.

## Typed-word replacement probe

```powershell
cargo run --bin replacement_probe
```

Keep the probe running, focus any Windows text field, and type one of the probe rules followed by a boundary such as Space, punctuation, Enter, or Tab:

- `дял` -> `для`
- `тчо` -> `что`
- `abcx` -> `ABC_REPLACED`

The fixed rules exist only in the probe executable. The library contains typed input, correction-decision, replacement-planning, and Windows execution contracts so the probe provider can later be replaced without rewriting the replacement path.

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