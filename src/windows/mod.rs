mod autocomplete_popup;
mod caret_locator;
mod clipboard_listener;
mod keyboard_runtime;
mod selected_text_runtime;

pub use autocomplete_popup::AutocompletePopupHandle;
pub use caret_locator::{CaretAnchor, CaretLocator, CaretSource, PopupPlacement, popup_placement};
pub use clipboard_listener::{ClipboardListenerError, ClipboardTextListener};
pub use keyboard_runtime::{
    InputProcessor, RuntimeDirective, RuntimeError, UndoDirective,
    request_global_keyboard_hook_stop, run_global_keyboard_hook,
};
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
