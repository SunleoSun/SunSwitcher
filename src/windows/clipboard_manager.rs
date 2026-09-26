use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use eframe::egui::{self, Color32, RichText, Sense, Stroke, vec2};
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

use crate::persistence::{ClipboardEntryContent, ClipboardEntryId, ClipboardEntryView, Database};

use super::keyboard_runtime::{RuntimeError, inject_selected_text};

const WINDOW_WIDTH: f32 = 520.0;
const WINDOW_HEIGHT: f32 = 420.0;
const ENTRY_LIMIT: usize = 100;
const INSERT_FOCUS_DELAY: Duration = Duration::from_millis(45);
const WINDOW_BG: Color32 = Color32::from_rgb(248, 246, 232);
const PANEL_BG: Color32 = Color32::from_rgb(255, 253, 232);
const TEXT: Color32 = Color32::from_rgb(24, 24, 24);
const MUTED_TEXT: Color32 = Color32::from_rgb(76, 76, 76);
const SELECTED_TAB: Color32 = Color32::from_rgb(213, 236, 255);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardManagerTab {
    Current,
    Pinned,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ClipboardManagerState {
    visible: bool,
    active_tab: ClipboardManagerTab,
    target_window_id: usize,
    serial: u64,
}

impl Default for ClipboardManagerState {
    fn default() -> Self {
        Self {
            visible: false,
            active_tab: ClipboardManagerTab::Current,
            target_window_id: 0,
            serial: 0,
        }
    }
}

#[derive(Clone)]
pub struct ClipboardManagerHandle {
    state: Arc<RwLock<ClipboardManagerState>>,
    repaint_ctx: egui::Context,
}

pub(crate) fn replace_clipboard_manager_state(
    state: &RwLock<ClipboardManagerState>,
    next: ClipboardManagerState,
) -> bool {
    state.write().is_ok_and(|mut state| {
        if *state == next {
            false
        } else {
            *state = next;
            true
        }
    })
}

impl ClipboardManagerHandle {
    pub(crate) fn new(
        state: Arc<RwLock<ClipboardManagerState>>,
        repaint_ctx: egui::Context,
    ) -> Self {
        Self { state, repaint_ctx }
    }

    pub fn show(&self, active_tab: ClipboardManagerTab, target_window_id: usize) {
        let next = self
            .state
            .read()
            .map(|state| ClipboardManagerState {
                visible: true,
                active_tab,
                target_window_id,
                serial: state.serial.wrapping_add(1),
            })
            .unwrap_or(ClipboardManagerState {
                visible: true,
                active_tab,
                target_window_id,
                serial: 1,
            });
        if replace_clipboard_manager_state(&self.state, next) {
            self.repaint_ctx.request_repaint();
        }
    }
}

pub(crate) struct ClipboardManagerApp {
    database_path: PathBuf,
    state: Arc<RwLock<ClipboardManagerState>>,
    shown: bool,
    filter: String,
    settings_visible: bool,
    last_serial: u64,
}

impl ClipboardManagerApp {
    pub(crate) fn new(database_path: PathBuf, state: Arc<RwLock<ClipboardManagerState>>) -> Self {
        Self {
            database_path,
            state,
            shown: false,
            filter: String::new(),
            settings_visible: false,
            last_serial: 0,
        }
    }

    pub(crate) fn logic(&mut self, ctx: &egui::Context) -> bool {
        if self.shown && ctx.input(|input| input.viewport().close_requested()) {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.hide_shared(ctx);
            return false;
        }

        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if state.serial != self.last_serial {
            self.last_serial = state.serial;
            ctx.request_repaint();
        }
        self.set_visible(ctx, state.visible);
        state.visible
    }

    pub(crate) fn ui(&mut self, ui: &mut egui::Ui) -> bool {
        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if !state.visible {
            return false;
        }
        self.apply_style(ui.ctx());
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.hide_shared(ui.ctx());
            return true;
        }

        let entries = match self.load_entries(state.active_tab) {
            Ok(entries) => entries,
            Err(error) => {
                egui::Frame::new().fill(WINDOW_BG).show(ui, |ui| {
                    ui.colored_label(TEXT, format!("Clipboard history unavailable: {error}"));
                });
                return true;
            }
        };

        let quick_insert = self.quick_insert_index(ui.ctx(), entries.len());
        if let Some(index) = quick_insert
            && let Some(entry) = entries.get(index)
        {
            self.insert_entry(ui.ctx(), entry, state.target_window_id);
            return true;
        }

        egui::Frame::new()
            .fill(WINDOW_BG)
            .stroke(Stroke::new(1.0, Color32::from_rgb(192, 192, 176)))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                self.title_bar(ui);
                ui.add_space(6.0);
                self.tabs(ui, state.active_tab);
                ui.add_space(6.0);

                let available_height = (ui.available_height() - 40.0).max(80.0);
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, Color32::from_rgb(220, 216, 188)))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .max_height(available_height)
                            .show(ui, |ui| {
                                if entries.is_empty() {
                                    ui.colored_label(MUTED_TEXT, "Clipboard history is empty");
                                } else {
                                    for (index, entry) in entries.iter().enumerate() {
                                        self.entry_row(ui, index, entry, state.target_window_id);
                                    }
                                }
                            });
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Filter").color(TEXT));
                    let response = ui.add_sized(
                        [(ui.available_width() - 104.0).max(120.0), 22.0],
                        egui::TextEdit::singleline(&mut self.filter)
                            .hint_text("case-insensitive filter"),
                    );
                    if response.changed() {
                        ui.ctx().request_repaint();
                    }
                    if ui.button("Settings").clicked() {
                        self.settings_visible = true;
                    }
                });
            });
        self.show_settings(ui.ctx());
        true
    }

    fn title_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let drag = ui.add(
                egui::Label::new(RichText::new("SunSwitcher Clipboard").strong().color(TEXT))
                    .sense(Sense::drag()),
            );
            if drag.drag_started() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").on_hover_text("Hide to tray").clicked() {
                    self.hide_shared(ui.ctx());
                }
            });
        });
    }

    fn tabs(&self, ui: &mut egui::Ui, active_tab: ClipboardManagerTab) {
        ui.horizontal(|ui| {
            for (tab, label) in [
                (ClipboardManagerTab::Current, "Current"),
                (ClipboardManagerTab::Pinned, "Pinned"),
            ] {
                let selected = active_tab == tab;
                let text = RichText::new(label).color(TEXT);
                let button = egui::Button::new(text).fill(if selected {
                    SELECTED_TAB
                } else {
                    Color32::from_rgb(232, 230, 210)
                });
                if ui.add(button).clicked()
                    && let Ok(mut shared) = self.state.write()
                {
                    shared.active_tab = tab;
                    shared.serial = shared.serial.wrapping_add(1);
                }
            }
        });
    }

    fn entry_row(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        entry: &ClipboardEntryView,
        target_window_id: usize,
    ) {
        ui.horizontal(|ui| {
            let button_label = if index < 9 {
                (index + 1).to_string()
            } else {
                "Insert".to_owned()
            };
            if ui.button(button_label).clicked() {
                self.insert_entry(ui.ctx(), entry, target_window_id);
            }
            let text = entry_text(entry);
            let preview = preview_text(text);
            let label = egui::Label::new(RichText::new(preview).color(TEXT)).sense(Sense::click());
            if ui
                .add_sized([(ui.available_width() - 72.0).max(80.0), 22.0], label)
                .double_clicked()
            {
                self.insert_entry(ui.ctx(), entry, target_window_id);
            }
            let pinned = entry.pinned_at_ms().is_some();
            let pin_label = if pinned { "Unpin" } else { "Pin" };
            if ui.button(pin_label).clicked() {
                self.pin_entry(entry.id(), !pinned);
                ui.ctx().request_repaint();
            }
        });
    }

    fn quick_insert_index(&self, ctx: &egui::Context, entry_count: usize) -> Option<usize> {
        if ctx.egui_wants_keyboard_input() {
            return None;
        }
        let keys = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
            egui::Key::Num6,
            egui::Key::Num7,
            egui::Key::Num8,
            egui::Key::Num9,
        ];
        ctx.input(|input| {
            keys.iter()
                .enumerate()
                .find_map(|(index, key)| input.key_pressed(*key).then_some(index))
        })
        .filter(|index| *index < entry_count)
    }

    fn apply_style(&self, ctx: &egui::Context) {
        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let mut style = (*ctx.style_of(theme)).clone();
            style.visuals = egui::Visuals::light();
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.window_fill = WINDOW_BG;
            style.visuals.panel_fill = WINDOW_BG;
            style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
            style.visuals.widgets.inactive.fg_stroke.color = TEXT;
            style.visuals.widgets.hovered.fg_stroke.color = TEXT;
            ctx.set_style_of(theme, style);
        }
    }

    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if self.shown == visible {
            return;
        }
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                "SunSwitcher Clipboard".to_owned(),
            ));
            ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(
                WINDOW_WIDTH,
                WINDOW_HEIGHT,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
                200.0, 160.0,
            )));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        } else {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        }
        self.shown = visible;
    }

    fn hide_shared(&mut self, ctx: &egui::Context) {
        if let Ok(mut state) = self.state.write() {
            state.visible = false;
            state.serial = state.serial.wrapping_add(1);
        }
        self.set_visible(ctx, false);
        ctx.request_repaint();
    }

    fn load_entries(
        &self,
        active_tab: ClipboardManagerTab,
    ) -> Result<Vec<ClipboardEntryView>, String> {
        let database = Database::open(&self.database_path).map_err(|error| error.to_string())?;
        match active_tab {
            ClipboardManagerTab::Current => database
                .load_clipboard_current(&self.filter, ENTRY_LIMIT)
                .map_err(|error| error.to_string()),
            ClipboardManagerTab::Pinned => database
                .load_clipboard_pinned(&self.filter, ENTRY_LIMIT)
                .map_err(|error| error.to_string()),
        }
    }

    fn pin_entry(&self, entry_id: ClipboardEntryId, pinned: bool) {
        let Ok(database) = Database::open(&self.database_path) else {
            return;
        };
        let result = if pinned {
            database.pin_clipboard_entry(entry_id, now_ms()).map(|_| ())
        } else {
            database.unpin_clipboard_entry(entry_id).map(|_| ())
        };
        if let Err(error) = result {
            eprintln!("clipboard pin update skipped: {error}");
        }
    }

    fn insert_entry(
        &mut self,
        ctx: &egui::Context,
        entry: &ClipboardEntryView,
        target_window_id: usize,
    ) {
        let text = match entry.content() {
            ClipboardEntryContent::Text(text) => text.clone(),
        };
        let entry_id = entry.id();
        if let Ok(database) = Database::open(&self.database_path)
            && let Err(error) = database.mark_clipboard_entry_used(entry_id, now_ms())
        {
            eprintln!("clipboard entry usage update skipped: {error}");
        }

        self.hide_shared(ctx);
        if target_window_id != 0 {
            unsafe {
                SetForegroundWindow(target_window_id as HWND);
            }
            thread::sleep(INSERT_FOCUS_DELAY);
        }
        if let Err(error) = insert_clipboard_text(&text) {
            eprintln!("clipboard text insertion failed: {error:?}");
        }
    }

    fn show_settings(&mut self, ctx: &egui::Context) {
        if !self.settings_visible {
            return;
        }
        egui::Window::new("SunSwitcher settings")
            .open(&mut self.settings_visible)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label("Default hotkeys");
                ui.separator();
                ui.monospace("Clipboard Current: Ctrl+Shift+Num-");
                ui.monospace("Clipboard Pinned: Ctrl+Shift+Num+");
                ui.monospace("Double Shift: physical Shift x2");
                ui.monospace("Completion accept: Tab");
                ui.monospace("Completion next: Alt+Right");
                ui.monospace("Undo correction: Pause");
            });
    }
}

fn insert_clipboard_text(text: &str) -> Result<(), RuntimeError> {
    inject_selected_text(text)
}

fn entry_text(entry: &ClipboardEntryView) -> &str {
    match entry.content() {
        ClipboardEntryContent::Text(text) => text,
    }
}

fn preview_text(text: &str) -> String {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_CHARS: usize = 72;
    if collapsed.chars().count() <= MAX_CHARS {
        return collapsed;
    }
    let mut preview = collapsed.chars().take(MAX_CHARS).collect::<String>();
    preview.push('…');
    preview
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_collapses_and_truncates_text() {
        assert_eq!(preview_text("hello\nworld"), "hello world");
        let long = "a".repeat(90);
        assert_eq!(preview_text(&long).chars().count(), 73);
    }

    #[test]
    fn state_replacement_ignores_identical_state() {
        let state = RwLock::new(ClipboardManagerState::default());
        let next = ClipboardManagerState {
            visible: true,
            active_tab: ClipboardManagerTab::Pinned,
            target_window_id: 42,
            serial: 1,
        };
        assert!(replace_clipboard_manager_state(&state, next.clone()));
        assert!(!replace_clipboard_manager_state(&state, next));
    }
}
