#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("completion_popup_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::time::Duration;

    use eframe::egui::{self, Color32, RichText, ViewportBuilder, ViewportCommand, pos2, vec2};
    use sunswitcher::windows::{CaretLocator, CaretSource, popup_placement};

    const POPUP_WIDTH: f32 = 320.0;
    const POPUP_HEIGHT: f32 = 154.0;
    const CARET_GAP: f32 = 6.0;

    pub fn run() -> eframe::Result {
        let options = eframe::NativeOptions {
            viewport: ViewportBuilder::default()
                .with_title("SunSwitcher completion probe")
                .with_inner_size(vec2(POPUP_WIDTH, POPUP_HEIGHT))
                .with_resizable(false)
                .with_decorations(false)
                .with_taskbar(false)
                .with_active(false)
                .with_always_on_top()
                .with_mouse_passthrough(true),
            ..Default::default()
        };

        eframe::run_native(
            "SunSwitcher completion probe",
            options,
            Box::new(|_| Ok(Box::new(CompletionPopupProbe::default()))),
        )
    }

    struct CompletionPopupProbe {
        locator: CaretLocator,
        last_source: Option<CaretSource>,
    }

    impl Default for CompletionPopupProbe {
        fn default() -> Self {
            Self {
                locator: CaretLocator::new(),
                last_source: None,
            }
        }
    }

    impl eframe::App for CompletionPopupProbe {
        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            let ctx = ui.ctx().clone();
            if let Some(anchor) = self
                .locator
                .locate()
                .or_else(|| self.locator.fallback_anchor())
            {
                self.last_source = Some(anchor.source());
                let placement = popup_placement(anchor, POPUP_WIDTH, POPUP_HEIGHT, CARET_GAP);
                ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(
                    placement.x,
                    placement.y,
                )));
            }

            ui.set_min_size(vec2(POPUP_WIDTH - 20.0, POPUP_HEIGHT - 20.0));
            ui.label(RichText::new("SunSwitcher suggestions").strong());
            ui.add_space(4.0);
            for (index, text) in [
                "сделать",
                "сделать проект",
                "сделать это сейчас",
                "сделать правильно",
            ]
            .iter()
            .enumerate()
            {
                let text = if index == 0 {
                    RichText::new(*text).strong()
                } else {
                    RichText::new(*text)
                };
                ui.label(text);
            }
            ui.add_space(4.0);
            let source = match self.last_source {
                Some(CaretSource::UiAutomationCaret) => "UIA caret",
                Some(CaretSource::UiAutomationTextEdit) => "UIA text edit",
                Some(CaretSource::UiAutomationSelection) => "UIA selection",
                Some(CaretSource::MsaaCaret) => "MSAA caret",
                Some(CaretSource::Win32GuiThread) => "Win32 caret",
                Some(CaretSource::PointerFallback) => "pointer fallback",
                Some(CaretSource::ForegroundWindowFallback) => "window fallback",
                None => "caret unavailable",
            };
            ui.label(RichText::new(source).small().color(Color32::GRAY));

            ctx.request_repaint_after(Duration::from_millis(80));
        }
    }
}

#[cfg(target_os = "windows")]
fn main() -> eframe::Result {
    windows_probe::run()
}
