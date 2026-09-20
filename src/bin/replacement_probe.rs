#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("replacement_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use sunswitcher::correction::{
        Confidence, CorrectionCandidate, CorrectionCandidateProvider, CorrectionDecision,
        CorrectionEngine, ReplacementText,
    };
    use sunswitcher::input::{CompletedToken, InputBuffer, InputEvent, InputOutcome};
    use sunswitcher::replacement::ReplacementEngine;
    use sunswitcher::windows::{InputProcessor, RuntimeDirective, run_global_keyboard_hook};

    pub fn run() {
        println!("SunSwitcher replacement probe");
        println!("Rules: дял -> для | тчо -> что | abcx -> ABC_REPLACED");
        println!(
            "The hook is global. Focus another application and type a rule followed by space/punctuation/Enter."
        );
        println!("Stop with Ctrl+C in this console.");

        let processor = ProbeProcessor::new();
        if let Err(error) = run_global_keyboard_hook(processor) {
            eprintln!("replacement probe stopped: {error:?}");
            std::process::exit(1);
        }
    }

    struct ProbeProvider;

    impl CorrectionCandidateProvider for ProbeProvider {
        fn candidates(&self, token: &CompletedToken) -> Vec<CorrectionCandidate> {
            let replacement = match token.text() {
                "дял" => "для",
                "тчо" => "что",
                "abcx" => "ABC_REPLACED",
                _ => return Vec::new(),
            };
            vec![CorrectionCandidate::new(
                ReplacementText::try_new(replacement).expect("probe replacement is non-empty"),
                Confidence::CERTAIN,
            )]
        }
    }

    struct ProbeProcessor {
        input: InputBuffer,
        corrections: CorrectionEngine<ProbeProvider>,
        replacements: ReplacementEngine,
    }

    impl ProbeProcessor {
        fn new() -> Self {
            Self {
                input: InputBuffer::new(),
                corrections: CorrectionEngine::new(
                    ProbeProvider,
                    Confidence::try_new(0.95).expect("valid probe threshold"),
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
