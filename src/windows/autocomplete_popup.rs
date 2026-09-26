use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui::{self, ViewportBuilder, ViewportCommand, pos2, vec2};

use super::caret_locator::{CaretAnchor, CaretLocator, popup_placement};
use super::clipboard_manager::{
    ClipboardManagerApp, ClipboardManagerHandle, ClipboardManagerState, ClipboardManagerTab,
};
use super::tray_icon::TrayIcon;

const POPUP_WIDTH: f32 = 360.0;
const ROW_HEIGHT: f32 = 24.0;
const POPUP_PADDING: f32 = 12.0;
const CARET_GAP: f32 = 6.0;
const PARKED_POSITION: f32 = -32_000.0;

fn install_system_font(ctx: &egui::Context) -> std::io::Result<()> {
    let windows_dir = std::env::var_os("WINDIR")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "WINDIR is not set"))?;
    let fonts_dir = windows_dir.join("Fonts");

    for filename in ["segoeui.ttf", "arial.ttf", "tahoma.ttf"] {
        if let Ok(bytes) = std::fs::read(fonts_dir.join(filename)) {
            let font_name = "windows-ui".to_owned();
            let mut definitions = egui::FontDefinitions::empty();
            definitions.font_data.insert(
                font_name.clone(),
                Arc::new(egui::FontData::from_owned(bytes)),
            );
            definitions
                .families
                .insert(egui::FontFamily::Proportional, vec![font_name.clone()]);
            definitions
                .families
                .insert(egui::FontFamily::Monospace, vec![font_name]);
            ctx.set_fonts(definitions);
            return Ok(());
        }
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no supported Windows UI font found in %WINDIR%\\Fonts",
    ))
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PopupState {
    suggestions: Vec<String>,
    selected: usize,
}

#[derive(Debug, Clone, Copy)]
struct PopupLayout {
    position: egui::Pos2,
    size: egui::Vec2,
}

impl PopupLayout {
    fn approximately_matches(self, other: Self) -> bool {
        (self.position.x - other.position.x).abs() < 0.5
            && (self.position.y - other.position.y).abs() < 0.5
            && (self.size.x - other.size.x).abs() < 0.5
            && (self.size.y - other.size.y).abs() < 0.5
    }
}

pub struct AutocompletePopupHandle {
    state: Arc<RwLock<PopupState>>,
    clipboard: ClipboardManagerHandle,
    repaint_ctx: egui::Context,
    shutdown: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    _tray: TrayIcon,
}

fn replace_popup_state(state: &RwLock<PopupState>, next: PopupState) -> bool {
    state.write().is_ok_and(|mut state| {
        if *state == next {
            false
        } else {
            *state = next;
            true
        }
    })
}

impl AutocompletePopupHandle {
    pub fn start(database_path: PathBuf) -> Result<Self, String> {
        let state = Arc::new(RwLock::new(PopupState::default()));
        let clipboard_state = Arc::new(RwLock::new(ClipboardManagerState::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_clipboard_state = Arc::clone(&clipboard_state);
        let worker_shutdown = Arc::clone(&shutdown);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let options = eframe::NativeOptions {
                event_loop_builder: Some(Box::new(|builder| {
                    use winit::platform::windows::EventLoopBuilderExtWindows as _;
                    builder.with_any_thread(true);
                })),
                viewport: ViewportBuilder::default()
                    .with_title("SunSwitcher UI")
                    .with_inner_size(vec2(1.0, 1.0))
                    .with_position(pos2(PARKED_POSITION, PARKED_POSITION))
                    .with_resizable(false)
                    .with_decorations(false)
                    .with_taskbar(false)
                    .with_active(false)
                    .with_visible(false)
                    .with_always_on_top()
                    .with_mouse_passthrough(false)
                    .with_transparent(false),
                ..Default::default()
            };

            let startup_sender = ready_sender.clone();
            let result = eframe::run_native(
                "SunSwitcher UI",
                options,
                Box::new(move |creation_context| {
                    install_system_font(&creation_context.egui_ctx)?;
                    let repaint_ctx = creation_context.egui_ctx.clone();
                    let _ = startup_sender.send(Ok(repaint_ctx));
                    Ok(Box::new(AutocompletePopupApp {
                        state: worker_state,
                        clipboard: ClipboardManagerApp::new(database_path, worker_clipboard_state),
                        shutdown: worker_shutdown,
                        locator: CaretLocator::new(),
                        fallback_anchor: None,
                        shown: false,
                        last_layout: None,
                    }))
                }),
            );
            if let Err(error) = result {
                let _ = ready_sender.send(Err(error.to_string()));
            }
        });

        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(repaint_ctx)) => {
                let clipboard = ClipboardManagerHandle::new(clipboard_state, repaint_ctx.clone());
                let tray = TrayIcon::start(clipboard.clone());
                Ok(Self {
                    state,
                    clipboard,
                    repaint_ctx,
                    shutdown,
                    worker: Some(worker),
                    _tray: tray,
                })
            }
            Ok(Err(error)) => {
                let _ = worker.join();
                Err(error)
            }
            Err(error) => {
                shutdown.store(true, Ordering::Release);
                Err(format!("SunSwitcher UI did not initialize: {error}"))
            }
        }
    }

    pub fn update<'a>(&self, suggestions: impl IntoIterator<Item = &'a str>, selected: usize) {
        let suggestions = suggestions
            .into_iter()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let next = PopupState {
            selected: selected.min(suggestions.len().saturating_sub(1)),
            suggestions,
        };
        let changed = replace_popup_state(&self.state, next);
        if changed {
            self.repaint_ctx.request_repaint();
        }
    }

    pub fn hide(&self) {
        let changed = replace_popup_state(&self.state, PopupState::default());
        if changed {
            self.repaint_ctx.request_repaint();
        }
    }

    pub fn show_clipboard(&self, active_tab: ClipboardManagerTab, target_window_id: usize) {
        self.clipboard.show(active_tab, target_window_id);
    }
}

impl Drop for AutocompletePopupHandle {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        self.repaint_ctx.request_repaint();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct AutocompletePopupApp {
    state: Arc<RwLock<PopupState>>,
    clipboard: ClipboardManagerApp,
    shutdown: Arc<AtomicBool>,
    locator: CaretLocator,
    fallback_anchor: Option<CaretAnchor>,
    shown: bool,
    last_layout: Option<PopupLayout>,
}

impl AutocompletePopupApp {
    fn hide_popup(&mut self, ctx: &egui::Context) {
        self.fallback_anchor = None;
        self.last_layout = None;
        if self.shown {
            ctx.send_viewport_cmd(ViewportCommand::Visible(false));
            self.shown = false;
        }
    }

    fn prepare_popup_viewport(&mut self, ctx: &egui::Context) {
        ctx.send_viewport_cmd(ViewportCommand::Title("SunSwitcher suggestions".to_owned()));
        ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
        ctx.send_viewport_cmd(ViewportCommand::Resizable(false));
    }

    fn apply_layout(&mut self, ctx: &egui::Context, layout: PopupLayout) {
        let layout_changed = self
            .last_layout
            .is_none_or(|previous| !previous.approximately_matches(layout));
        if !self.shown || layout_changed {
            self.prepare_popup_viewport(ctx);
            ctx.send_viewport_cmd(ViewportCommand::InnerSize(layout.size));
            ctx.send_viewport_cmd(ViewportCommand::OuterPosition(layout.position));
            self.last_layout = Some(layout);
        }
        if !self.shown {
            ctx.send_viewport_cmd(ViewportCommand::Visible(true));
            self.shown = true;
        }
    }
}

impl eframe::App for AutocompletePopupApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.shutdown.load(Ordering::Acquire) {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }

        if self.clipboard.logic(ctx) {
            self.fallback_anchor = None;
            self.last_layout = None;
            self.shown = false;
            return;
        }

        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if state.suggestions.is_empty() {
            self.hide_popup(ctx);
            return;
        }

        let anchor = if let Some(anchor) = self.locator.locate() {
            self.fallback_anchor = None;
            anchor
        } else {
            if self.fallback_anchor.is_none() {
                self.fallback_anchor = self.locator.fallback_anchor();
            }
            let Some(anchor) = self.fallback_anchor else {
                self.hide_popup(ctx);
                return;
            };
            anchor
        };

        let popup_height = POPUP_PADDING * 2.0 + ROW_HEIGHT * state.suggestions.len() as f32;
        let placement = popup_placement(anchor, POPUP_WIDTH, popup_height, CARET_GAP);
        self.apply_layout(
            ctx,
            PopupLayout {
                position: pos2(placement.x, placement.y),
                size: vec2(POPUP_WIDTH, popup_height),
            },
        );
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.shutdown.load(Ordering::Acquire) {
            return;
        }
        if self.clipboard.ui(ui) {
            return;
        }

        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if state.suggestions.is_empty() {
            return;
        }

        let bounds = ui.max_rect();
        ui.painter()
            .rect_filled(bounds, 6.0, egui::Color32::from_rgb(30, 30, 30));

        let row_width = POPUP_WIDTH - POPUP_PADDING * 2.0;
        let origin = bounds.left_top() + vec2(POPUP_PADDING, POPUP_PADDING);
        for (index, suggestion) in state.suggestions.iter().enumerate() {
            let row = egui::Rect::from_min_size(
                origin + vec2(0.0, ROW_HEIGHT * index as f32),
                vec2(row_width, ROW_HEIGHT),
            );
            if index == state.selected {
                ui.painter()
                    .rect_filled(row, 4.0, egui::Color32::from_rgb(62, 76, 96));
            }
            ui.painter().text(
                row.left_center() + vec2(8.0, 0.0),
                egui::Align2::LEFT_CENTER,
                suggestion,
                egui::FontId::proportional(15.0),
                egui::Color32::WHITE,
            );
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        let channel = 30.0 / 255.0;
        [channel, channel, channel, 1.0]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn popup_state_change_detection_ignores_identical_updates() {
        let state = RwLock::new(PopupState::default());
        let project = PopupState {
            suggestions: vec!["project".to_owned()],
            selected: 0,
        };
        assert!(replace_popup_state(&state, project.clone()));
        assert!(!replace_popup_state(&state, project));

        let expanded = PopupState {
            suggestions: vec!["project".to_owned(), "probe".to_owned()],
            selected: 1,
        };
        assert!(replace_popup_state(&state, expanded.clone()));
        assert_eq!(*state.read().expect("popup state lock"), expanded);
        assert!(replace_popup_state(&state, PopupState::default()));
        assert!(!replace_popup_state(&state, PopupState::default()));
    }
}
