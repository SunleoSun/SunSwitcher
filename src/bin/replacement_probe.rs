#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("replacement_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::path::PathBuf;
    use std::time::{Instant, SystemTime, UNIX_EPOCH};

    use sunswitcher::adaptive::{
        AdaptiveCompletionSession, AdaptiveCorrectionDirective, AdaptiveCorrectionSession,
        AdaptiveLexicalRuntime,
    };
    use sunswitcher::completion::{
        CompletionApplyOutcome, CompletionCommand, CompletionCommandResult,
    };
    use sunswitcher::correction::Confidence;
    use sunswitcher::input::InputEvent;
    use sunswitcher::language::{KeyboardLayoutSwitch, switch_keyboard_layout_text};
    use sunswitcher::persistence::{Database, UndoHotkey};
    use sunswitcher::replacement::{ReplacementOutcome, UndoOutcome, UndoReplacementAction};
    use sunswitcher::windows::{AutocompletePopupHandle, ClipboardTextListener};
    use sunswitcher::windows::{ClipboardCommand, ClipboardManagerTab, ObservableClipboardContent};
    use sunswitcher::windows::{
        InputProcessor, RuntimeDirective, UndoDirective, request_global_keyboard_hook_stop,
        run_global_keyboard_hook,
    };
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };
    #[cfg(test)]
    const MAX_CLIPBOARD_LEARNING_CHARS: usize = 4096;
    #[cfg(test)]
    const MAX_CLIPBOARD_LEARNING_LINES: usize = 128;

    pub fn run() {
        println!("SunSwitcher replacement probe");
        println!("Bilingual lexical provider: Russian + English, including wrong-layout typos.");
        println!("Examples: дял/ддля/ддляя/lkz/llkz/lzk -> для | hlelo/helllo/руддщ -> hello");
        println!(
            "The hook is global. Focusing/clicking discards stale tracked text; the first newly typed character starts a fresh token immediately. Type an example followed by Space/Enter/Tab."
        );
        println!(
            "Pause: without a selection, undo the previous correction; with a selected word, remove it from learned user_words."
        );
        println!(
            "Autocomplete: after 3 typed letters; Up/Down selects, Tab accepts all, Alt+Right accepts one word, Del removes the selected prediction, Esc closes. Double Shift switches the selected text or previous word between keyboard layouts."
        );
        println!("Stop with Ctrl+C in this console.");

        let Ok(_console_handler) = ConsoleControlHandler::install() else {
            eprintln!("Could not install the probe shutdown handler.");
            return;
        };

        let (processor, undo_hotkey) = match ProbeProcessor::new() {
            Ok(processor) => processor,
            Err(error) => {
                eprintln!("Could not load the probe language snapshot from SQLite: {error}");
                return;
            }
        };
        if let Err(error) = run_global_keyboard_hook(processor, undo_hotkey) {
            eprintln!("replacement probe stopped: {error:?}");
            return;
        }
        println!("replacement probe stopped; global keyboard and mouse hooks were released");
    }

    struct ConsoleControlHandler;

    impl ConsoleControlHandler {
        fn install() -> Result<Self, ()> {
            let installed = unsafe { SetConsoleCtrlHandler(Some(console_control_handler), 1) };
            (installed != 0).then_some(Self).ok_or(())
        }
    }

    impl Drop for ConsoleControlHandler {
        fn drop(&mut self) {
            unsafe {
                SetConsoleCtrlHandler(Some(console_control_handler), 0);
            }
        }
    }

    unsafe extern "system" fn console_control_handler(control_type: u32) -> i32 {
        if matches!(control_type, CTRL_C_EVENT | CTRL_BREAK_EVENT)
            && request_global_keyboard_hook_stop()
        {
            return 1;
        }
        0
    }

    struct ProbeProcessor {
        _clipboard_listener: ClipboardTextListener,
        runtime: AdaptiveLexicalRuntime,
        session: AdaptiveCorrectionSession,
        completion: AdaptiveCompletionSession,
        popup: AutocompletePopupHandle,
        // Clipboard manager state lives in the shared UI runtime.
    }

    impl ProbeProcessor {
        fn new() -> Result<(Self, UndoHotkey), String> {
            let local_app_data = std::env::var_os("LOCALAPPDATA")
                .ok_or_else(|| "LOCALAPPDATA is not available".to_owned())?;
            let app_data_dir = PathBuf::from(local_app_data).join("SunSwitcher");
            std::fs::create_dir_all(&app_data_dir)
                .map_err(|error| format!("could not create app-data directory: {error}"))?;
            let database_path = app_data_dir.join("sunswitcher.db");
            let database = Database::open(&database_path).map_err(|error| error.to_string())?;
            let undo_hotkey = database
                .settings()
                .map_err(|error| error.to_string())?
                .undo_hotkey();
            let minimum_confidence = Confidence::try_new(0.80).expect("valid probe threshold");
            let runtime = AdaptiveLexicalRuntime::start(database, minimum_confidence)
                .map_err(|error| error.to_string())?;
            let learning = runtime.learning();
            let popup = AutocompletePopupHandle::start(database_path.clone())
                .map_err(|error| format!("SunSwitcher UI failed: {error}"))?;
            let clipboard_updates = popup.clipboard_handle();
            let clipboard_database_path = database_path.clone();
            let clipboard_listener = ClipboardTextListener::start(move |content| {
let started_at = Instant::now();
let observed_at_ms = now_ms();
let profile = ClipboardContentProfile::from(&content);
let mut history_ms = 0;
let mut prune_ms = 0;
let mut learning_queue_ms = 0;
let mut adaptive_learning = "skipped";
let mut history_changed = false;

let history_started_at = Instant::now();
match Database::open(&clipboard_database_path) {
Ok(database) => {
match &content {
ObservableClipboardContent::Text(text) => {
match database.record_clipboard_text(text, observed_at_ms) {
Ok(_) => history_changed = true,
Err(error) => eprintln!("clipboard history skipped: {error}"),
}
}
ObservableClipboardContent::Image(image) => {
match database.record_clipboard_image(
image.format(),
image.data(),
observed_at_ms,
) {
Ok(_) => history_changed = true,
Err(error) => eprintln!("clipboard image history skipped: {error}"),
}
}
}
history_ms = elapsed_ms(history_started_at);
let prune_started_at = Instant::now();
if let Ok(settings) = database.settings()
&& let Err(error) = database.prune_clipboard_history(settings.clipboard_history_limit())
{
eprintln!("clipboard history prune skipped: {error}");
}
prune_ms = elapsed_ms(prune_started_at);
}
Err(error) => eprintln!("clipboard history unavailable: {error}"),
}
if history_changed {
clipboard_updates.notify_history_changed();
}

if let ObservableClipboardContent::Text(text) = &content {
let learning_started_at = Instant::now();
match learning.observe_text(text.as_str(), observed_at_ms) {
Ok(()) => adaptive_learning = "queued",
Err(error) => eprintln!("clipboard learning skipped: {error}"),
}
learning_queue_ms = elapsed_ms(learning_started_at);
}

let total_ms = elapsed_ms(started_at);
eprintln!(
"[clipboard-profile] kind={} bytes={} chars={} lines={} history_ms={} prune_ms={} learning_queue_ms={} total_ms={} adaptive_learning={}",
profile.kind,
profile.bytes,
profile.chars,
profile.lines,
history_ms,
prune_ms,
learning_queue_ms,
total_ms,
adaptive_learning
);
})
.map_err(|error| format!("clipboard listener failed: {error:?}"))?;
            let session = runtime.live_session();
            let completion = runtime.completion_session();
            println!("Adaptive state: {}", database_path.display());
            Ok((
                Self {
                    _clipboard_listener: clipboard_listener,
                    runtime,
                    session,
                    completion,
                    popup,
                    // Clipboard manager is owned by popup,
                },
                undo_hotkey,
            ))
        }
    }

    impl ProbeProcessor {
        fn sync_popup(&self) {
            self.popup.update(
                self.completion
                    .suggestions()
                    .iter()
                    .map(|suggestion| suggestion.text()),
                self.completion.selected_index(),
            );
        }
    }

    struct ClipboardContentProfile {
        kind: String,
        bytes: usize,
        chars: usize,
        lines: usize,
    }

    impl ClipboardContentProfile {
        fn from(content: &ObservableClipboardContent) -> Self {
            match content {
                ObservableClipboardContent::Text(text) => Self {
                    kind: "text".to_owned(),
                    bytes: text.len(),
                    chars: text.chars().count(),
                    lines: text.lines().count(),
                },
                ObservableClipboardContent::Image(image) => Self {
                    kind: image.format().to_owned(),
                    bytes: image.data().len(),
                    chars: 0,
                    lines: 0,
                },
            }
        }
    }

    fn elapsed_ms(started_at: Instant) -> u128 {
        started_at.elapsed().as_millis()
    }

    #[cfg(test)]
    fn should_learn_clipboard_text(text: &str) -> bool {
        text.chars().take(MAX_CLIPBOARD_LEARNING_CHARS + 1).count() <= MAX_CLIPBOARD_LEARNING_CHARS
            && text.lines().take(MAX_CLIPBOARD_LEARNING_LINES + 1).count()
                <= MAX_CLIPBOARD_LEARNING_LINES
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    impl InputProcessor for ProbeProcessor {
        fn completion_active(&self) -> bool {
            self.completion.is_active()
        }

        fn switch_layout_text(&self, text: &str) -> Option<KeyboardLayoutSwitch> {
            let snapshot = self.runtime.snapshots().load().ok()?;
            switch_keyboard_layout_text(snapshot.languages(), text)
        }
        fn tracked_layout_switch_text(&self) -> Option<String> {
            let text = self.session.current_layout_switch_span();
            (!text.is_empty()).then(|| text.to_owned())
        }

        fn tracked_layout_switch_applied(&mut self, text: &str) {
            let observed_at_ms = now_ms();
            self.session.tracked_layout_switch_applied(text);
            if let Err(error) = self.completion.layout_switch_applied(text, observed_at_ms) {
                eprintln!("adaptive completion refresh after layout switch skipped: {error}");
            }
            self.sync_popup();
        }

        fn process(&mut self, event: InputEvent) -> RuntimeDirective {
            let observed_at_ms = now_ms();
            if let Err(error) = self.completion.process_event(event, observed_at_ms) {
                eprintln!("adaptive completion skipped: {error}");
            }
            let directive = match self.session.process(event, observed_at_ms) {
                Ok(AdaptiveCorrectionDirective::Pass) => RuntimeDirective::Pass,
                Ok(AdaptiveCorrectionDirective::Replace(action)) => {
                    println!("[REPLACE] -> {:?}", action.replacement().as_str());
                    RuntimeDirective::Replace(action)
                }
                Ok(AdaptiveCorrectionDirective::ReplaceLivePrefix(action)) => {
                    println!(
                        "[LIVE REPLACE] {:?} -> {:?}",
                        action.original_visible(),
                        action.replacement().as_str()
                    );
                    RuntimeDirective::ReplaceLivePrefix(action)
                }
                Err(error) => {
                    eprintln!("adaptive correction skipped: {error}");
                    RuntimeDirective::Pass
                }
            };
            if let Some(canonical) = self.session.take_resolved_completion_word()
                && let Err(error) = self
                    .completion
                    .layout_switch_applied(&canonical, observed_at_ms)
            {
                eprintln!("adaptive completion refresh after correction skipped: {error}");
            }
            self.sync_popup();
            directive
        }

        fn replacement_outcome(&mut self, outcome: ReplacementOutcome) {
            if let Err(error) = self.session.replacement_outcome(outcome) {
                eprintln!("adaptive correction outcome was not persisted: {error}");
            }
            if outcome == ReplacementOutcome::Applied
                && let Some(canonical) = self.session.take_resolved_completion_word()
                && let Err(error) = self.completion.layout_switch_applied(&canonical, now_ms())
            {
                eprintln!("adaptive completion refresh after correction outcome skipped: {error}");
            }
            self.sync_popup();
        }

        fn live_prefix_replacement_outcome(&mut self, outcome: ReplacementOutcome) {
            if let Err(error) = self.session.live_prefix_replacement_outcome(outcome) {
                eprintln!("adaptive live-prefix correction outcome was not persisted: {error}");
            }
            if outcome == ReplacementOutcome::Applied
                && let Some(prefix) = self.session.take_resolved_live_prefix()
                && let Err(error) = self.completion.live_prefix_replaced(&prefix, now_ms())
            {
                eprintln!(
                    "adaptive completion refresh after live-prefix correction skipped: {error}"
                );
            }
            self.sync_popup();
        }
        fn delete_user_word(&mut self, text: &str) {
            if let Err(error) = self.runtime.learning().delete_user_word(text.to_owned()) {
                eprintln!("user-word deletion skipped: {error}");
                return;
            }

            let observed_at_ms = now_ms();
            if let Err(error) = self.session.process(InputEvent::Invalidate, observed_at_ms) {
                eprintln!("adaptive correction reset after user-word deletion skipped: {error}");
            }
            if let Err(error) = self
                .completion
                .process_event(InputEvent::Invalidate, observed_at_ms)
            {
                eprintln!("adaptive completion reset after user-word deletion skipped: {error}");
            }
            self.sync_popup();
            println!("[USER WORD DELETE] {:?}", text);
        }

        fn undo(&mut self) -> UndoDirective {
            match self.session.request_undo(now_ms()) {
                Ok(Some(action)) => {
                    match &action {
                        UndoReplacementAction::Completed(action) => {
                            println!("[UNDO] -> {:?}", action.replacement().as_str());
                        }
                        UndoReplacementAction::LivePrefix(action) => {
                            println!("[UNDO LIVE] -> {:?}", action.replacement().as_str());
                        }
                    }
                    UndoDirective::Restore(action)
                }
                Ok(None) => UndoDirective::Pass,
                Err(error) => {
                    eprintln!("adaptive undo skipped: {error}");
                    UndoDirective::Pass
                }
            }
        }

        fn undo_outcome(&mut self, outcome: UndoOutcome) {
            if let Err(error) = self.session.undo_outcome(outcome) {
                eprintln!("adaptive undo outcome was not persisted: {error}");
            }
            if outcome == UndoOutcome::Applied
                && let Some(canonical) = self.session.take_resolved_completion_word()
                && let Err(error) = self.completion.layout_switch_applied(&canonical, now_ms())
            {
                eprintln!("adaptive completion refresh after undo outcome skipped: {error}");
            }
            if outcome == UndoOutcome::Applied
                && let Some(prefix) = self.session.take_resolved_live_prefix()
                && let Err(error) = self.completion.live_prefix_replaced(&prefix, now_ms())
            {
                eprintln!("adaptive completion refresh after live-prefix undo skipped: {error}");
            }
            self.sync_popup();
        }

        fn completion_command(&mut self, command: CompletionCommand) -> CompletionCommandResult {
            let result = match self.completion.command(command) {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("adaptive completion command skipped: {error}");
                    CompletionCommandResult::Consumed
                }
            };
            match &result {
                CompletionCommandResult::AcceptSuffix(suffix) => {
                    println!("[COMPLETE] +{:?}", suffix);
                }
                CompletionCommandResult::AcceptWord(word) => {
                    println!("[COMPLETE WORD] +{:?}", word);
                }
                _ => {}
            }
            self.sync_popup();
            result
        }

        fn clipboard_command(&mut self, command: ClipboardCommand, target_window_id: usize) {
            let (tab, command_label) = match command {
                ClipboardCommand::OpenCurrent => (ClipboardManagerTab::Current, "open_current"),
                ClipboardCommand::OpenPinned => (ClipboardManagerTab::Pinned, "open_pinned"),
            };
            eprintln!(
                "[hotkey] clipboard command received command={} target_window_id={}",
                command_label, target_window_id
            );
            self.popup.show_clipboard(tab, target_window_id);
        }

        fn completion_suffix_outcome(&mut self, outcome: CompletionApplyOutcome) {
            let observed_at_ms = now_ms();
            if let Err(error) = self
                .completion
                .suffix_acceptance_outcome(outcome, observed_at_ms)
            {
                eprintln!("adaptive completion suffix outcome skipped: {error}");
            }
            self.sync_popup();
        }

        fn completion_word_outcome(&mut self, outcome: CompletionApplyOutcome) {
            let observed_at_ms = now_ms();
            if outcome == CompletionApplyOutcome::Applied
                && let Err(error) = self.session.process(InputEvent::Invalidate, observed_at_ms)
            {
                eprintln!("adaptive correction reset after completion skipped: {error}");
            }
            if let Err(error) = self
                .completion
                .word_acceptance_outcome(outcome, observed_at_ms)
            {
                eprintln!("adaptive completion word outcome skipped: {error}");
            }
            self.sync_popup();
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn clipboard_learning_guard_accepts_short_text_and_rejects_large_text() {
            assert!(should_learn_clipboard_text("hello world"));

            let exact_char_limit = "a".repeat(MAX_CLIPBOARD_LEARNING_CHARS);
            assert!(should_learn_clipboard_text(&exact_char_limit));

            let long_text = "a".repeat(MAX_CLIPBOARD_LEARNING_CHARS + 1);
            assert!(!should_learn_clipboard_text(&long_text));

            let exact_line_limit = (0..MAX_CLIPBOARD_LEARNING_LINES)
                .map(|_| "x")
                .collect::<Vec<_>>()
                .join("\n");
            assert!(should_learn_clipboard_text(&exact_line_limit));

            let many_lines = (0..=MAX_CLIPBOARD_LEARNING_LINES)
                .map(|_| "x")
                .collect::<Vec<_>>()
                .join("\n");
            assert!(!should_learn_clipboard_text(&many_lines));
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}
