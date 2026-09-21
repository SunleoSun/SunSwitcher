#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("replacement_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::time::{SystemTime, UNIX_EPOCH};

    use sunswitcher::adaptive::{
        AdaptiveCorrectionDirective, AdaptiveCorrectionSession, AdaptiveLexicalRuntime,
    };
    use sunswitcher::correction::Confidence;
    use sunswitcher::input::InputEvent;
    use sunswitcher::persistence::{Database, UndoHotkey};
    use sunswitcher::replacement::{ReplacementOutcome, UndoOutcome};
    use sunswitcher::windows::ClipboardTextListener;
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
        println!("Undo hotkey: Pause (PS), with no modifiers.");
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
        _runtime: AdaptiveLexicalRuntime,
        session: AdaptiveCorrectionSession,
    }

    impl ProbeProcessor {
        fn new() -> Result<(Self, UndoHotkey), String> {
            let database = Database::open_in_memory().map_err(|error| error.to_string())?;
            let undo_hotkey = database
                .settings()
                .map_err(|error| error.to_string())?
                .undo_hotkey();
            let minimum_confidence = Confidence::try_new(0.80).expect("valid probe threshold");
            let runtime = AdaptiveLexicalRuntime::start(database, minimum_confidence)
                .map_err(|error| error.to_string())?;
            let learning = runtime.learning();
            let clipboard_listener = ClipboardTextListener::start(move |text| {
                if let Err(error) = learning.observe_text(text, now_ms()) {
                    eprintln!("clipboard learning skipped: {error}");
                }
            })
            .map_err(|error| format!("clipboard listener failed: {error:?}"))?;
            let session = runtime.session();
            Ok((
                Self {
                    _clipboard_listener: clipboard_listener,
                    _runtime: runtime,
                    session,
                },
                undo_hotkey,
            ))
        }
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0)
    }

    impl InputProcessor for ProbeProcessor {
        fn process(&mut self, event: InputEvent) -> RuntimeDirective {
            match self.session.process(event, now_ms()) {
                Ok(AdaptiveCorrectionDirective::Pass) => RuntimeDirective::Pass,
                Ok(AdaptiveCorrectionDirective::Replace(action)) => {
                    println!("[REPLACE] -> {:?}", action.replacement().as_str());
                    RuntimeDirective::Replace(action)
                }
                Err(error) => {
                    eprintln!("adaptive correction skipped: {error}");
                    RuntimeDirective::Pass
                }
            }
        }

        fn replacement_outcome(&mut self, outcome: ReplacementOutcome) {
            if let Err(error) = self.session.replacement_outcome(outcome) {
                eprintln!("adaptive correction outcome was not persisted: {error}");
            }
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
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}
