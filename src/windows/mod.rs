mod autocomplete_popup;
mod caret_locator;
mod clipboard_listener;
mod clipboard_manager;
mod hotkey_capture;
mod keyboard_runtime;
mod selected_text_runtime;
mod tray_icon;

pub use autocomplete_popup::AutocompletePopupHandle;
pub use caret_locator::{CaretAnchor, CaretLocator, CaretSource, PopupPlacement, popup_placement};
pub use clipboard_listener::{ClipboardListenerError, ClipboardTextListener};
pub use clipboard_manager::ClipboardManagerTab;
pub use hotkey_capture::HotkeyCaptureHandle;
pub use keyboard_runtime::{
    ClipboardCommand, IgnoreWordStatus, InputProcessor, RuntimeDirective, RuntimeError,
    UndoDirective, request_global_keyboard_hook_stop, run_global_keyboard_hook,
};
pub use selected_text_runtime::{ClipboardImagePayload, ObservableClipboardContent};
pub use selected_text_runtime::{SelectedTextRuntimeError, SelectedTextSession};

#[cfg(test)]
mod clipboard_listener_certification;
#[cfg(test)]
mod keyboard_runtime_certification;
#[cfg(test)]
mod keyboard_runtime_tests;
#[cfg(test)]
mod selected_text_runtime_certification;
#[cfg(test)]
mod selected_text_runtime_tests;
