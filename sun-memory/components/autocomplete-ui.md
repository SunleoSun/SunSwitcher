---
description: Maps Windows caret anchoring, live egui autocomplete popup, default completion hotkeys, and suffix insertion; read before changing popup placement, caret discovery, focus behavior, or autocomplete UI integration.
---
# Autocomplete UI and caret anchoring

## Current boundary

`src/windows/caret_locator.rs` owns discovery of an on-screen text-caret anchor. It returns a typed `CaretAnchor` with screen-space coordinates, caret height, and the backend that produced it. Completion policy and ranking do not belong here.

The current caret discovery order is:

1. UI Automation `TextPattern2::GetCaretRange`; the focused UIA element is tried first, then up to eight raw-view ancestors, then a bounded raw-view descendant search (depth 8 / 128 elements) because browsers, terminals, and composite editors may expose the text provider deeper in the accessibility subtree;
2. UI Automation `TextEditPattern`, converted through its inherited `TextPattern`, with the same focused/ancestor/bounded-descendant discovery;
3. UI Automation text selection, including adjacent-character recovery for degenerate caret ranges;
4. MSAA `OBJID_CARET` + `IAccessible::accLocation`, tried first on the foreground GUI thread's focused HWND and then on the top-level foreground HWND, before classic Win32 caret discovery;
5. Win32 `GetGUIThreadInfo` + `ClientToScreen` for classic caret-owning controls.

`CaretLocator::locate()` remains the preferred anchor path used by the live autocomplete popup. If UIA/MSAA/Win32 cannot expose a caret while suggestions are active, production now uses `fallback_anchor()` as a final fail-soft placement: current pointer first, foreground-window geometry second. That fallback is captured once for the active suggestion session and is not recomputed every frame, so the popup does not chase the mouse; a subsequently discovered real caret immediately takes ownership again.

## Placement

`popup_placement` places the popup below the proven caret with a small gap, clamps it to the nearest monitor work area, and flips above the caret if there is insufficient vertical space below. UIA caret ranges that expose no rectangle are recovered from the adjacent character range so Chromium/Electron-style degenerate carets can still produce screen geometry. Caret discovery and popup placement remain separate behaviors.

## egui popup

`src/windows/autocomplete_popup.rs` owns the runtime suggestion window. It runs a borderless, taskbar-hidden, always-on-top egui/Glow viewport with `active=false` and explicit opaque native composition. The viewport is kept paintable instead of using native hidden/show transitions: while there are no suggestions it is a 1x1 window parked far off-screen. State updates request repaint through the retained `egui::Context`; with suggestions, `App::logic` prefers a real caret anchor and otherwise uses the frozen final fallback described above, then resizes and moves the already-live viewport through `popup_placement`. Rendering paints an opaque dark background, selected-row fill, and white text, with matching native clear color. Because this build intentionally uses `eframe` with `default-features = false`, the popup installs a Windows system font at startup (Segoe UI, then Arial/Tahoma fallback) into egui's proportional and monospace families. Without an explicit font definition, egui still paints shapes but text glyphs are absent. Popup startup fails instead of silently showing a textless window if none of the supported Windows fonts can be loaded. Native `mouse_passthrough` is intentionally disabled for this Glow window: winit implements cursor hit-test disabling on Windows with `WS_EX_TRANSPARENT | WS_EX_LAYERED`, and the layered style was observed to expose an unpainted/transparent OpenGL client area instead of the egui frame. The popup therefore prioritizes correct opaque rendering and non-activation at creation over native click-through. The popup owns a dedicated Windows UI thread; its eframe event loop uses `EventLoopBuilderExtWindows::with_any_thread(true)`. `replacement_probe` drives this popup from the typed completion session, and `src/bin/completion_popup_probe.rs` remains the lower-level diagnostic probe that can display caret/fallback source information.

## Input and hotkeys

Live completion normally activates after three characters in the current token. Unmodified Up/Down changes selection; unmodified Enter **or Tab** accepts the complete selected suggestion; Alt+Right accepts exactly the next word plus its separating space; unmodified Delete removes the selected prediction; and unmodified Esc dismisses. After a proven Alt+Right insertion, completion tracking advances to the accepted word and sequence suggestions refresh immediately. The remainder that was visible in the selected row before acceptance is protected as the prefix of the refreshed top row: ranking may extend it with newly predicted words, but may not change those already-visible words. Repeated Alt+Right therefore never inserts an unseen replacement word; Up/Down deliberately releases that protection and chooses another path. While completion is active, the physical Alt modifier used to form Alt+Right is owned by SunSwitcher: its down/up pair is suppressed from downstream hooks, while the internal keyboard state still records Alt so Right can resolve to `AcceptNextWord`. The accepted word is injected as Unicode only; no synthetic Alt release/repress is sent to the foreground application. Delete removes the row immediately from the current session; learned multi-word rows delete their exact `text_history` source, while single-word candidates persist a completion-only suppression so dictionary/user lexical authority is not destroyed. Keyboard runtime resolves a completion command before calling the downstream hook chain: handled command keydown/up pairs are SunSwitcher-only; only `Pass` from an inactive/non-consuming completion session is forwarded. Double Shift is a separate layout-switch command: two otherwise plain **physical** Shift taps within 400 ms trigger on the second Shift release; SunSwitcher/foreign injected keyboard transitions never participate in the gesture. The selected/previous text transform now returns both transformed text and its target language. The Windows effect path explicitly replaces an active captured selection with Delete+Unicode insertion rather than relying on every control to replace a selection during `SendInput`. If a terminal collapses the temporary previous-word selection while servicing the clipboard fallback, SunSwitcher normalizes back to the captured word with Left → Ctrl+Right → Ctrl+Shift+Left, deletes that exact selection, then inserts the transformed text; it no longer blind-backspaces by the captured character count and therefore cannot consume the word before it merely because a selection survived unexpectedly. Only after the text effect succeeds does SunSwitcher request the matching installed foreground keyboard layout through `WM_INPUTLANGCHANGEREQUEST`, so the next physical keystroke uses the same language as the converted text. These bindings are defaults only; persistent configurability belongs to the future settings UI.

## Validation anchors

- `src/windows/caret_locator.rs`: caret fallback and monitor-aware placement.
- `src/windows/autocomplete_popup.rs`: live focusless egui suggestion window.
- `src/windows/keyboard_runtime.rs`: typed default completion hotkey mapping and suffix injection.
- `src/bin/completion_popup_probe.rs`: static caret/popup integration probe.
- `src/bin/replacement_probe.rs`: current end-to-end live completion integration.

## Related memory

- `components/completion`
- `components/adaptive-correction`
- `main/core-project-principles`
