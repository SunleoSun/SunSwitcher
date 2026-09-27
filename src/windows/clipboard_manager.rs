use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::{Arc, RwLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, RichText, Sense, Stroke, TextureOptions,
    ViewportBuilder, ViewportId, pos2, vec2,
};
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
};
use windows_sys::Win32::System::Threading::GetCurrentProcessId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    SetForegroundWindow,
};

use crate::persistence::{ClipboardEntryContent, ClipboardEntryId, ClipboardEntryView, Database};

use super::clipboard_listener::InternalClipboardMutationGuard;
use super::keyboard_runtime::{RuntimeError, inject_ctrl_chord};
use super::selected_text_runtime::{set_image_clipboard_payload, set_unicode_clipboard};

const WINDOW_WIDTH: f32 = 520.0;
const WINDOW_HEIGHT: f32 = 420.0;
const PREVIEW_WIDTH: f32 = WINDOW_WIDTH;
const PREVIEW_HEIGHT: f32 = WINDOW_HEIGHT;
const PREVIEW_GAP: f32 = 8.0;
const PREVIEW_HIDE_DELAY: Duration = Duration::from_secs(1);
const PARKED_POSITION: f32 = -32_000.0;
const ENTRY_LIMIT: usize = 100;
const INSERT_FOCUS_DELAY: Duration = Duration::from_millis(45);
const ENTRY_ROW_HEIGHT: f32 = 26.0;
const NUMBER_BUTTON_WIDTH: f32 = 28.0;
const TAB_BUTTON_WIDTH: f32 = 74.0;
const TAB_BUTTON_HEIGHT: f32 = 24.0;
const TEXT_AVERAGE_WIDTH: f32 = 7.0;
const WINDOW_BG: Color32 = Color32::from_rgb(30, 32, 38);
const PANEL_BG: Color32 = Color32::from_rgb(18, 20, 25);
const ROW_BG: Color32 = Color32::from_rgb(23, 25, 30);
const ROW_HOVER_BG: Color32 = Color32::from_rgb(38, 41, 48);
const TEXT: Color32 = Color32::from_rgb(244, 242, 226);
const MUTED_TEXT: Color32 = Color32::from_rgb(170, 168, 150);
const ACCENT: Color32 = Color32::from_rgb(226, 184, 63);
const ACCENT_DIM: Color32 = Color32::from_rgb(104, 84, 32);
const SELECTED_TAB: Color32 = Color32::from_rgb(77, 63, 28);
const INACTIVE_TAB: Color32 = Color32::from_rgb(42, 43, 40);
const CLIPBOARD_WINDOW_TITLE: &str = "SunSwitcher Clipboard";
const TITLE_BAR_BG_COLOR: u32 = windows_colorref_rgb(36, 38, 45);
const TITLE_BAR_TEXT_COLOR: u32 = windows_colorref_rgb(244, 242, 226);

const fn windows_colorref_rgb(red: u8, green: u8, blue: u8) -> u32 {
    red as u32 | ((green as u32) << 8) | ((blue as u32) << 16)
}

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
        let _ = replace_clipboard_manager_state(&self.state, next);
        self.repaint_ctx
            .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.repaint_ctx
            .send_viewport_cmd(egui::ViewportCommand::Focus);
        self.repaint_ctx.request_repaint();
    }
}

pub(crate) struct ClipboardManagerApp {
    database_path: PathBuf,
    state: Arc<RwLock<ClipboardManagerState>>,
    shown: bool,
    filter: String,
    settings_visible: bool,
    last_serial: u64,
    preview: Arc<RwLock<Option<ClipboardPreview>>>,
    preview_hovered_at: Arc<RwLock<Option<Instant>>>,
    preview_shown: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryActivation {
    CopyOnly,
    InsertIntoTarget,
}

#[derive(Debug, Clone)]
struct ClipboardPreview {
    entry: ClipboardEntryView,
    last_hovered: Instant,
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
            preview: Arc::new(RwLock::new(None)),
            preview_hovered_at: Arc::new(RwLock::new(None)),
            preview_shown: false,
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
        if state.visible {
            apply_clipboard_title_bar_color();
        }
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
        if let Some(tab) = self.consume_tab_switch_shortcuts(ui.ctx()) {
            self.set_active_tab(tab);
        }
        self.expire_preview(ui.ctx());

        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
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
            self.activate_entry(
                ui.ctx(),
                entry,
                state.target_window_id,
                EntryActivation::InsertIntoTarget,
            );
            return true;
        }

        egui::Frame::new()
            .fill(WINDOW_BG)
            .stroke(Stroke::new(1.0, ACCENT_DIM))
            .inner_margin(egui::Margin::same(8))
            .show(ui, |ui| {
                self.tabs(ui, state.active_tab);
                ui.add_space(6.0);

                let available_height = (ui.available_height() - 40.0).max(80.0);
                egui::Frame::new()
                    .fill(PANEL_BG)
                    .stroke(Stroke::new(1.0, ACCENT_DIM))
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
                    let gear_width = 28.0;
                    let spacing = ui.spacing().item_spacing.x;
                    let response = ui.add_sized(
                        [
                            (ui.available_width() - gear_width - spacing).max(120.0),
                            22.0,
                        ],
                        egui::TextEdit::singleline(&mut self.filter),
                    );
                    if response.has_focus() {
                        ui.painter().rect_stroke(
                            response.rect.expand(1.0),
                            3.0,
                            Stroke::new(1.5, ACCENT),
                            egui::StrokeKind::Inside,
                        );
                    }
                    if response.changed() {
                        ui.ctx().request_repaint();
                    }
                    if gear_button(ui).clicked() {
                        self.settings_visible = true;
                    }
                });
            });
        self.show_settings(ui.ctx());
        self.show_preview(ui.ctx());
        true
    }

    fn consume_tab_switch_shortcuts(&self, ctx: &egui::Context) -> Option<ClipboardManagerTab> {
        let modifiers = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        let pinned = ctx.input_mut(|input| {
            input.consume_key(modifiers, egui::Key::Plus)
                || input.consume_key(modifiers, egui::Key::Equals)
        });
        if pinned {
            return Some(ClipboardManagerTab::Pinned);
        }
        let current = ctx.input_mut(|input| input.consume_key(modifiers, egui::Key::Minus));
        current.then_some(ClipboardManagerTab::Current)
    }

    fn set_active_tab(&self, tab: ClipboardManagerTab) {
        if let Ok(mut shared) = self.state.write()
            && shared.active_tab != tab
        {
            shared.active_tab = tab;
            shared.serial = shared.serial.wrapping_add(1);
        }
    }

    fn expire_preview(&mut self, ctx: &egui::Context) {
        if self
            .preview_last_activity()
            .is_some_and(|last_activity| last_activity.elapsed() > PREVIEW_HIDE_DELAY)
        {
            self.clear_preview(ctx);
        }
    }

    fn set_preview_entry(&mut self, ctx: &egui::Context, entry: ClipboardEntryView) {
        let mut changed = false;
        if let Ok(mut preview) = self.preview.write() {
            match preview.as_mut() {
                Some(current) if current.entry.id() == entry.id() => {
                    current.last_hovered = Instant::now();
                }
                _ => {
                    *preview = Some(ClipboardPreview {
                        entry,
                        last_hovered: Instant::now(),
                    });
                    changed = true;
                }
            }
        }
        if changed {
            ctx.request_repaint();
            ctx.request_repaint_of(preview_viewport_id());
        }
        ctx.request_repaint_after(Duration::from_millis(150));
    }

    fn clear_preview(&mut self, ctx: &egui::Context) {
        if let Ok(mut preview) = self.preview.write() {
            *preview = None;
        }
        if let Ok(mut hovered_at) = self.preview_hovered_at.write() {
            *hovered_at = None;
        }
        self.hide_preview_viewport(ctx);
        ctx.request_repaint_of(preview_viewport_id());
    }

    fn preview_last_activity(&self) -> Option<Instant> {
        let row_hover = self
            .preview
            .read()
            .ok()
            .and_then(|preview| preview.as_ref().map(|preview| preview.last_hovered));
        let window_hover = self
            .preview_hovered_at
            .read()
            .ok()
            .and_then(|hovered_at| *hovered_at);
        match (row_hover, window_hover) {
            (Some(row_hover), Some(window_hover)) => Some(row_hover.max(window_hover)),
            (Some(row_hover), None) => Some(row_hover),
            (None, Some(window_hover)) => Some(window_hover),
            (None, None) => None,
        }
    }

    fn tabs(&self, ui: &mut egui::Ui, active_tab: ClipboardManagerTab) {
        ui.horizontal(|ui| {
            for (tab, label) in [
                (ClipboardManagerTab::Current, "Current"),
                (ClipboardManagerTab::Pinned, "Pinned"),
            ] {
                let selected = active_tab == tab;
                let button = egui::Button::new(RichText::new(label).color(TEXT))
                    .fill(if selected { SELECTED_TAB } else { INACTIVE_TAB })
                    .stroke(Stroke::new(1.0, if selected { ACCENT } else { ACCENT_DIM }));
                if ui
                    .add_sized([TAB_BUTTON_WIDTH, TAB_BUTTON_HEIGHT], button)
                    .clicked()
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
        let mut activation = None;
        let mut pin_action = None;
        ui.horizontal(|ui| {
            if index < 9 {
                let button =
                    egui::Button::new(RichText::new((index + 1).to_string()).color(ACCENT))
                        .fill(ROW_BG)
                        .stroke(Stroke::new(1.0, ACCENT));
                if ui
                    .add_sized([NUMBER_BUTTON_WIDTH, ENTRY_ROW_HEIGHT], button)
                    .clicked()
                {
                    activation = Some(EntryActivation::InsertIntoTarget);
                }
            } else {
                ui.add_space(NUMBER_BUTTON_WIDTH + ui.spacing().item_spacing.x);
            }

            let summary = entry_summary(entry);
            let (response, text_truncated) = entry_text_response(ui, &summary);
            if response.hovered() {
                if entry_has_preview(entry, text_truncated) {
                    self.set_preview_entry(ui.ctx(), entry.clone());
                } else {
                    self.clear_preview(ui.ctx());
                }
            }
            let clicked = response.clicked();
            let pinned = entry.pinned_at_ms().is_some();
            let pin_label = if pinned { "Unpin" } else { "Pin" };
            response.context_menu(|ui| {
                if ui.button(pin_label).clicked() {
                    pin_action = Some(!pinned);
                    ui.close();
                }
            });
            if clicked {
                activation = Some(EntryActivation::CopyOnly);
            }
        });

        if let Some(pinned) = pin_action {
            self.pin_entry(entry.id(), pinned);
            ui.ctx().request_repaint();
        }
        if let Some(activation) = activation {
            self.activate_entry(ui.ctx(), entry, target_window_id, activation);
        }
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
            style.visuals = egui::Visuals::dark();
            style.visuals.override_text_color = Some(TEXT);
            style.visuals.window_fill = WINDOW_BG;
            style.visuals.panel_fill = WINDOW_BG;
            style.visuals.extreme_bg_color = PANEL_BG;
            style.visuals.faint_bg_color = ROW_BG;
            style.spacing.scroll = egui::style::ScrollStyle::solid();
            style.visuals.selection.bg_fill = SELECTED_TAB;
            style.visuals.selection.stroke.color = TEXT;
            style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
            style.visuals.widgets.inactive.bg_fill = ROW_BG;
            style.visuals.widgets.inactive.weak_bg_fill = ROW_BG;
            style.visuals.widgets.inactive.fg_stroke.color = TEXT;
            style.visuals.widgets.hovered.bg_fill = ACCENT_DIM;
            style.visuals.widgets.hovered.weak_bg_fill = ACCENT_DIM;
            style.visuals.widgets.hovered.fg_stroke.color = TEXT;
            style.visuals.widgets.open.bg_fill = ACCENT;
            style.visuals.widgets.open.weak_bg_fill = ACCENT_DIM;
            style.visuals.widgets.active.bg_fill = ACCENT;
            style.visuals.widgets.active.fg_stroke.color = TEXT;
            ctx.set_style_of(theme, style);
        }
    }

    fn set_visible(&mut self, ctx: &egui::Context, visible: bool) {
        if self.shown == visible {
            return;
        }
        if visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(
                CLIPBOARD_WINDOW_TITLE.to_owned(),
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
            park_hidden_root_viewport(ctx);
        }
        self.shown = visible;
    }

    fn hide_shared(&mut self, ctx: &egui::Context) {
        if let Ok(mut state) = self.state.write() {
            state.visible = false;
            state.serial = state.serial.wrapping_add(1);
        }
        if let Ok(mut preview) = self.preview.write() {
            *preview = None;
        }
        self.hide_preview_viewport(ctx);
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

    fn mark_entry_used(&self, entry_id: ClipboardEntryId) {
        if let Ok(database) = Database::open(&self.database_path)
            && let Err(error) = database.mark_clipboard_entry_used(entry_id, now_ms())
        {
            eprintln!("clipboard entry usage update skipped: {error}");
        }
    }

    fn activate_entry(
        &mut self,
        ctx: &egui::Context,
        entry: &ClipboardEntryView,
        target_window_id: usize,
        activation: EntryActivation,
    ) {
        match entry.content() {
            ClipboardEntryContent::Text(text) => {
                if let Err(error) = copy_clipboard_text(text) {
                    eprintln!("clipboard text copy failed: {error}");
                    return;
                }
                self.hide_shared(ctx);
                if activation == EntryActivation::InsertIntoTarget {
                    if target_window_id == 0 {
                        eprintln!("clipboard text insertion skipped: no target window");
                        return;
                    }
                    let focused = unsafe { SetForegroundWindow(target_window_id as HWND) } != 0;
                    if !focused {
                        eprintln!(
                            "clipboard text insertion skipped: target window could not be focused"
                        );
                        return;
                    }
                    thread::sleep(INSERT_FOCUS_DELAY);
                    if let Err(error) = insert_clipboard_text() {
                        eprintln!("clipboard text insertion failed: {error:?}");
                    }
                }
            }
            ClipboardEntryContent::Image { format, data } => {
                if let Err(error) = copy_clipboard_image(format, data) {
                    eprintln!("clipboard image copy failed: {error}");
                    return;
                }
                self.mark_entry_used(entry.id());
                self.hide_shared(ctx);
            }
        }
    }

    fn show_preview(&mut self, ctx: &egui::Context) {
        let has_preview = self
            .preview
            .read()
            .ok()
            .is_some_and(|preview| preview.is_some());
        if !has_preview {
            self.hide_preview_viewport(ctx);
            return;
        }

        let position = preview_window_position(ctx, vec2(PREVIEW_WIDTH, PREVIEW_HEIGHT));
        let preview_state = Arc::clone(&self.preview);
        let hover_state = Arc::clone(&self.preview_hovered_at);
        let viewport = ViewportBuilder::default()
            .with_title("SunSwitcher Clipboard Preview")
            .with_inner_size(vec2(PREVIEW_WIDTH, PREVIEW_HEIGHT))
            .with_position(position)
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(false)
            .with_visible(true);
        ctx.show_viewport_deferred(preview_viewport_id(), viewport, move |ui, _class| {
            apply_preview_style(ui.ctx());
            let bounds = ui.max_rect();
            if ui.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|position| bounds.contains(position))
            }) {
                if let Ok(mut hovered_at) = hover_state.write() {
                    *hovered_at = Some(Instant::now());
                }
                ui.ctx().request_repaint_after(Duration::from_millis(150));
            }
            if let Some(preview) = preview_state
                .read()
                .ok()
                .and_then(|preview| preview.as_ref().cloned())
            {
                preview_entry_ui(ui, &preview.entry);
            }
        });
        self.preview_shown = true;
        ctx.request_repaint_after(Duration::from_millis(150));
    }

    fn hide_preview_viewport(&mut self, ctx: &egui::Context) {
        if self.preview_shown {
            ctx.send_viewport_cmd_to(preview_viewport_id(), egui::ViewportCommand::Visible(false));
            self.preview_shown = false;
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

fn entry_text_response(ui: &mut egui::Ui, text: &str) -> (egui::Response, bool) {
    let width = ui.available_width().max(80.0);
    let (rect, response) = ui.allocate_exact_size(vec2(width, ENTRY_ROW_HEIGHT), Sense::click());
    let fill = if response.hovered() {
        ROW_HOVER_BG
    } else {
        ROW_BG
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 2.0, fill);
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        Stroke::new(1.0, ACCENT_DIM),
    );
    let max_chars = preview_capacity_for_width((width - 12.0).max(24.0));
    let (text, truncated) = preview_text(text, max_chars);
    painter.text(
        rect.left_center() + vec2(6.0, 0.0),
        Align2::LEFT_CENTER,
        text,
        FontId::proportional(13.0),
        TEXT,
    );
    (response, truncated)
}

fn gear_button(ui: &mut egui::Ui) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 22.0), Sense::click());
    let fill = if response.hovered() {
        ROW_HOVER_BG
    } else {
        WINDOW_BG
    };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, fill);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, ACCENT_DIM),
        egui::StrokeKind::Inside,
    );
    let center = rect.center();
    let outer = 6.5;
    let inner = 2.6;
    for step in 0..8 {
        let angle = step as f32 * std::f32::consts::TAU / 8.0;
        let direction = vec2(angle.cos(), angle.sin());
        painter.line_segment(
            [
                center + direction * (outer - 1.5),
                center + direction * (outer + 2.0),
            ],
            Stroke::new(1.5, ACCENT),
        );
    }
    painter.circle_stroke(center, outer, Stroke::new(1.5, ACCENT));
    painter.circle_stroke(center, inner, Stroke::new(1.4, ACCENT));
    response
}

fn copy_clipboard_text(text: &str) -> Result<(), String> {
    set_unicode_clipboard(text).map_err(|error| format!("{error:?}"))
}

fn insert_clipboard_text() -> Result<(), RuntimeError> {
    inject_ctrl_chord(b'V' as u16)
}

fn entry_summary(entry: &ClipboardEntryView) -> String {
    match entry.content() {
        ClipboardEntryContent::Text(text) => text.clone(),
        ClipboardEntryContent::Image { format, .. } => format!(
            "🖼 Image · {format} · {}",
            format_timestamp_ms(entry.last_seen_at_ms())
        ),
    }
}

fn preview_capacity_for_width(width: f32) -> usize {
    ((width / TEXT_AVERAGE_WIDTH).floor() as usize).max(8)
}

fn entry_has_preview(entry: &ClipboardEntryView, text_truncated: bool) -> bool {
    matches!(entry.content(), ClipboardEntryContent::Image { .. }) || text_truncated
}

fn park_hidden_root_viewport(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Title("SunSwitcher UI".to_owned()));
    ctx.send_viewport_cmd(egui::ViewportCommand::Decorations(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(false));
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(1.0, 1.0)));
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(
        PARKED_POSITION,
        PARKED_POSITION,
    )));
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
}

fn preview_text(text: &str, max_chars: usize) -> (String, bool) {
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let char_count = collapsed.chars().count();
    if char_count <= max_chars {
        return (collapsed, false);
    }
    let visible_chars = max_chars.saturating_sub(3).max(1);
    let mut preview = collapsed.chars().take(visible_chars).collect::<String>();
    preview.push_str("...");
    (preview, true)
}

fn copy_clipboard_image(format: &str, data: &[u8]) -> Result<(), String> {
    let _guard = InternalClipboardMutationGuard::begin();
    set_image_clipboard_payload(format, data).map_err(|error| format!("{error:?}"))
}

fn preview_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("sunswitcher_clipboard_preview")
}
fn preview_window_position(ctx: &egui::Context, size: egui::Vec2) -> egui::Pos2 {
    ctx.input(|input| {
        let viewport = input.viewport();
        let Some(root) = viewport.outer_rect.or(viewport.inner_rect) else {
            return pos2(740.0, 160.0);
        };
        let monitor = viewport.monitor_size.unwrap_or(vec2(1920.0, 1080.0));
        let right = root.right() + PREVIEW_GAP;
        let left = root.left() - size.x - PREVIEW_GAP;
        let x = if right + size.x <= monitor.x {
            right
        } else if left >= 0.0 {
            left
        } else {
            (monitor.x - size.x - PREVIEW_GAP).max(PREVIEW_GAP)
        };
        let y = root.top().clamp(
            PREVIEW_GAP,
            (monitor.y - size.y - PREVIEW_GAP).max(PREVIEW_GAP),
        );
        pos2(x, y)
    })
}

fn apply_preview_style(ctx: &egui::Context) {
    for theme in [egui::Theme::Light, egui::Theme::Dark] {
        let mut style = (*ctx.style_of(theme)).clone();
        style.visuals = egui::Visuals::dark();
        style.visuals.override_text_color = Some(TEXT);
        style.visuals.window_fill = WINDOW_BG;
        style.visuals.panel_fill = PANEL_BG;
        style.visuals.extreme_bg_color = PANEL_BG;
        style.visuals.faint_bg_color = ROW_BG;
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.visuals.selection.bg_fill = SELECTED_TAB;
        style.visuals.selection.stroke.color = TEXT;
        style.visuals.widgets.noninteractive.fg_stroke.color = TEXT;
        style.visuals.widgets.inactive.bg_fill = ROW_BG;
        style.visuals.widgets.inactive.weak_bg_fill = ROW_BG;
        style.visuals.widgets.inactive.fg_stroke.color = TEXT;
        style.visuals.widgets.hovered.bg_fill = ACCENT_DIM;
        style.visuals.widgets.hovered.weak_bg_fill = ACCENT_DIM;
        style.visuals.widgets.hovered.fg_stroke.color = TEXT;
        style.visuals.widgets.open.bg_fill = ACCENT;
        style.visuals.widgets.open.weak_bg_fill = ACCENT_DIM;
        style.visuals.widgets.active.bg_fill = ACCENT;
        style.visuals.widgets.active.fg_stroke.color = TEXT;
        ctx.set_style_of(theme, style);
    }
}

fn apply_clipboard_title_bar_color() {
    let Some(hwnd) = find_current_process_window(CLIPBOARD_WINDOW_TITLE) else {
        return;
    };
    unsafe {
        let dark_mode: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE as u32,
            (&dark_mode as *const i32).cast(),
            size_of::<i32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR as u32,
            (&TITLE_BAR_BG_COLOR as *const u32).cast(),
            size_of::<u32>() as u32,
        );
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TEXT_COLOR as u32,
            (&TITLE_BAR_TEXT_COLOR as *const u32).cast(),
            size_of::<u32>() as u32,
        );
    }
}

struct WindowSearch {
    process_id: u32,
    title: Vec<u16>,
    hwnd: HWND,
}

fn find_current_process_window(title: &str) -> Option<HWND> {
    let mut search = WindowSearch {
        process_id: unsafe { GetCurrentProcessId() },
        title: title.encode_utf16().collect(),
        hwnd: null_mut(),
    };
    unsafe {
        EnumWindows(
            Some(enum_current_process_window),
            (&mut search as *mut WindowSearch) as LPARAM,
        );
    }
    (!search.hwnd.is_null()).then_some(search.hwnd)
}

unsafe extern "system" fn enum_current_process_window(hwnd: HWND, lparam: LPARAM) -> i32 {
    if unsafe { IsWindowVisible(hwnd) } == 0 {
        return 1;
    }
    let search = unsafe { &mut *(lparam as *mut WindowSearch) };
    let mut process_id = 0u32;
    unsafe {
        GetWindowThreadProcessId(hwnd, &mut process_id);
    }
    if process_id != search.process_id {
        return 1;
    }
    let title_len = unsafe { GetWindowTextLengthW(hwnd) };
    if title_len <= 0 {
        return 1;
    }
    let mut title = vec![0u16; title_len as usize + 1];
    let copied = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
    if copied <= 0 {
        return 1;
    }
    if title[..copied as usize] == search.title[..] {
        search.hwnd = hwnd;
        return 0;
    }
    1
}
fn preview_entry_ui(ui: &mut egui::Ui, entry: &ClipboardEntryView) {
    egui::Frame::new()
        .fill(PANEL_BG)
        .stroke(Stroke::new(1.0, ACCENT))
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| match entry.content() {
            ClipboardEntryContent::Text(text) => {
                ui.push_id(("clipboard-preview-text", entry.id().get()), |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format_timestamp_ms(entry.last_seen_at_ms()))
                                .color(MUTED_TEXT),
                        )
                        .selectable(true),
                    );
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.add(
                                egui::Label::new(RichText::new(text).color(TEXT)).selectable(true),
                            );
                        });
                });
            }
            ClipboardEntryContent::Image { format, data } => {
                ui.label(
                    RichText::new(format!(
                        "Image · {format} · {}",
                        format_timestamp_ms(entry.last_seen_at_ms())
                    ))
                    .color(MUTED_TEXT),
                );
                ui.separator();
                if let Some(image) = dib_to_color_image(data) {
                    let texture = ui.ctx().load_texture(
                        format!("clipboard-preview-{}", entry.id().get()),
                        image,
                        TextureOptions::LINEAR,
                    );
                    let available = ui.available_size();
                    ui.image((texture.id(), available));
                } else {
                    ui.label(
                        RichText::new("Image preview is unavailable for this clipboard format")
                            .color(TEXT),
                    );
                }
            }
        });
}

fn dib_to_color_image(data: &[u8]) -> Option<ColorImage> {
    if data.len() < 40 {
        return None;
    }
    let header_size = u32::from_le_bytes(data.get(0..4)?.try_into().ok()?) as usize;
    if header_size < 40 || data.len() <= header_size {
        return None;
    }
    let width = i32::from_le_bytes(data.get(4..8)?.try_into().ok()?);
    let height = i32::from_le_bytes(data.get(8..12)?.try_into().ok()?);
    let planes = u16::from_le_bytes(data.get(12..14)?.try_into().ok()?);
    let bits_per_pixel = u16::from_le_bytes(data.get(14..16)?.try_into().ok()?);
    let compression = u32::from_le_bytes(data.get(16..20)?.try_into().ok()?);
    if planes != 1 || width == 0 || height == 0 || !matches!(bits_per_pixel, 24 | 32) {
        return None;
    }
    let pixel_offset = match compression {
        0 => header_size,
        3 if bits_per_pixel == 32 => {
            let masks_offset = 40usize;
            let red_mask =
                u32::from_le_bytes(data.get(masks_offset..masks_offset + 4)?.try_into().ok()?);
            let green_mask = u32::from_le_bytes(
                data.get(masks_offset + 4..masks_offset + 8)?
                    .try_into()
                    .ok()?,
            );
            let blue_mask = u32::from_le_bytes(
                data.get(masks_offset + 8..masks_offset + 12)?
                    .try_into()
                    .ok()?,
            );
            if (red_mask, green_mask, blue_mask) != (0x00ff_0000, 0x0000_ff00, 0x0000_00ff) {
                return None;
            }
            if header_size == 40 {
                header_size.checked_add(12)?
            } else {
                header_size
            }
        }
        _ => return None,
    };
    let width = width.unsigned_abs() as usize;
    let height_abs = height.unsigned_abs() as usize;
    let top_down = height < 0;
    let bytes_per_pixel = usize::from(bits_per_pixel / 8);
    let row_stride = (width * usize::from(bits_per_pixel)).div_ceil(32) * 4;
    let needed = pixel_offset.checked_add(row_stride.checked_mul(height_abs)?)?;
    if data.len() < needed {
        return None;
    }
    let mut pixels = Vec::with_capacity(width * height_abs);
    for y in 0..height_abs {
        let source_y = if top_down { y } else { height_abs - 1 - y };
        let row = pixel_offset + source_y * row_stride;
        for x in 0..width {
            let offset = row + x * bytes_per_pixel;
            let b = *data.get(offset)?;
            let g = *data.get(offset + 1)?;
            let r = *data.get(offset + 2)?;
            let a = if bits_per_pixel == 32 {
                *data.get(offset + 3)?
            } else {
                255
            };
            pixels.push(Color32::from_rgba_unmultiplied(r, g, b, a));
        }
    }
    Some(ColorImage::new([width, height_abs], pixels))
}

fn format_timestamp_ms(timestamp_ms: i64) -> String {
    let seconds = timestamp_ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3600;
    let minute = (seconds_of_day % 3600) / 60;
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m, d)
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
        assert_eq!(
            preview_text("hello\nworld", 72),
            ("hello world".to_owned(), false)
        );
        let long = "a".repeat(90);
        assert_eq!(preview_text(&long, 12), ("aaaaaaaaa...".to_owned(), true));
        assert_eq!(preview_text(&long, 2), ("a...".to_owned(), true));
    }

    #[test]
    fn preview_capacity_has_a_small_width_floor() {
        assert_eq!(preview_capacity_for_width(1.0), 8);
        assert!(preview_capacity_for_width(200.0) > 8);
    }

    #[test]
    fn dib_decoder_handles_32_bit_bottom_up_rows() {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&[3, 2, 1, 255]);
        let image = dib_to_color_image(&dib).unwrap();
        assert_eq!(image.size, [1, 1]);
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_unmultiplied(1, 2, 3, 255)
        );
    }

    #[test]
    fn dib_decoder_handles_bitfields_masks_before_pixels() {
        let mut dib = Vec::new();
        dib.extend_from_slice(&40u32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1i32.to_le_bytes());
        dib.extend_from_slice(&1u16.to_le_bytes());
        dib.extend_from_slice(&32u16.to_le_bytes());
        dib.extend_from_slice(&3u32.to_le_bytes());
        dib.extend_from_slice(&4u32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0i32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0u32.to_le_bytes());
        dib.extend_from_slice(&0x00ff_0000u32.to_le_bytes());
        dib.extend_from_slice(&0x0000_ff00u32.to_le_bytes());
        dib.extend_from_slice(&0x0000_00ffu32.to_le_bytes());
        dib.extend_from_slice(&[3, 2, 1, 255]);

        let image = dib_to_color_image(&dib).unwrap();
        assert_eq!(image.size, [1, 1]);
        assert_eq!(
            image.pixels[0],
            Color32::from_rgba_unmultiplied(1, 2, 3, 255)
        );
    }

    #[test]
    fn timestamp_formats_epoch_minutes() {
        assert_eq!(format_timestamp_ms(0), "1970-01-01 00:00");
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
