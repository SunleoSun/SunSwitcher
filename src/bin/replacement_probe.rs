#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("replacement_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

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
    use sunswitcher::replacement::{ReplacementOutcome, UndoOutcome};
    use sunswitcher::windows::{AutocompletePopupHandle, ClipboardTextListener};
    use sunswitcher::windows::{ClipboardCommand, ClipboardManagerTab};
    use sunswitcher::windows::{
        InputProcessor, RuntimeDirective, UndoDirective, request_global_keyboard_hook_stop,
        run_global_keyboard_hook,
    };
    use windows_sys::Win32::System::Console::{
        CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler,
    };

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
            let clipboard_database_path = database_path.clone();
            let clipboard_listener = ClipboardTextListener::start(move |text| {
                let observed_at_ms = now_ms();
                match Database::open(&clipboard_database_path) {
                    Ok(database) => {
                        if let Err(error) = database.record_clipboard_text(&text, observed_at_ms) {
                            eprintln!("clipboard history skipped: {error}");
                        }
                        if let Ok(settings) = database.settings()
                            && let Err(error) =
                                database.prune_clipboard_history(settings.clipboard_history_limit())
                        {
                            eprintln!("clipboard history prune skipped: {error}");
                        }
                    }
                    Err(error) => eprintln!("clipboard history unavailable: {error}"),
                }
                if let Err(error) = learning.observe_text(text, observed_at_ms) {
                    eprintln!("clipboard learning skipped: {error}");
                }
            })
            .map_err(|error| format!("clipboard listener failed: {error:?}"))?;
            let session = runtime.session();
            let completion = runtime.completion_session();
            let popup = AutocompletePopupHandle::start(database_path.clone())
                .map_err(|error| format!("SunSwitcher UI failed: {error}"))?;
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
            if let Err(error) = self
                .completion
                .process_event(InputEvent::Invalidate, observed_at_ms)
            {
                eprintln!("adaptive completion reset after layout switch skipped: {error}");
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
                Err(error) => {
                    eprintln!("adaptive correction skipped: {error}");
                    RuntimeDirective::Pass
                }
            };
            if let Some(canonical) = self.session.take_resolved_completion_word() {
                self.completion.canonicalize_last_context_word(&canonical);
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
            {
                self.completion.canonicalize_last_context_word(&canonical);
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
                    println!("[UNDO] -> {:?}", action.replacement().as_str());
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
            {
                self.completion.canonicalize_last_context_word(&canonical);
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
            let tab = match command {
                ClipboardCommand::OpenCurrent => ClipboardManagerTab::Current,
                ClipboardCommand::OpenPinned => ClipboardManagerTab::Pinned,
            };
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
}

#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}
