#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("replacement_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use sunswitcher::correction::{
        Confidence, CorrectionDecision, CorrectionEngine, LexicalCorrectionProvider,
    };
    use sunswitcher::input::{InputBuffer, InputEvent, InputOutcome};
    use sunswitcher::language::{english_language_pack, russian_language_pack};
    use sunswitcher::replacement::ReplacementEngine;
    use sunswitcher::windows::{
        InputProcessor, RuntimeDirective, request_global_keyboard_hook_stop,
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
            "The hook is global. After focusing/clicking another application, press Space once to establish a safe token boundary, then type an example followed by Space/Enter/Tab."
        );
        println!("Stop with Ctrl+C in this console.");

        let Ok(_console_handler) = ConsoleControlHandler::install() else {
            eprintln!("Could not install the probe shutdown handler.");
            return;
        };

        let processor = ProbeProcessor::new();
        if let Err(error) = run_global_keyboard_hook(processor) {
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
        input: InputBuffer,
        corrections: CorrectionEngine<LexicalCorrectionProvider>,
        replacements: ReplacementEngine,
    }

    impl ProbeProcessor {
        fn new() -> Self {
            Self {
                input: InputBuffer::new(),
                corrections: CorrectionEngine::new(
                    LexicalCorrectionProvider::try_new(vec![
                        russian_language_pack(),
                        english_language_pack(),
                    ])
                    .expect("probe has configured languages"),
                    Confidence::try_new(0.80).expect("valid probe threshold"),
                ),
                replacements: ReplacementEngine::new(),
            }
        }
    }

    impl InputProcessor for ProbeProcessor {
        fn process(&mut self, event: InputEvent) -> RuntimeDirective {
            let InputOutcome::Completed(token) = self.input.process(event) else {
                return RuntimeDirective::Pass;
            };

            let decision = self.corrections.decide(&token);
            if matches!(decision, CorrectionDecision::Keep) {
                return RuntimeDirective::Pass;
            }

            let Some(action) = self.replacements.plan(&token, decision) else {
                return RuntimeDirective::Pass;
            };
            println!(
                "[REPLACE] {:?} -> {:?}",
                token.text(),
                action.replacement().as_str()
            );
            RuntimeDirective::Replace(action)
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}
