use std::mem::size_of;
use std::path::PathBuf;
use std::ptr::null_mut;
use std::sync::{Arc, Mutex, RwLock};
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
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, VK_CANCEL, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_PAUSE, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsWindowVisible,
    SetForegroundWindow,
};

use crate::persistence::{
    AppSettings, ClipboardEntryContent, ClipboardEntryId, ClipboardEntryList, ClipboardEntryView,
    ClipboardReorderPosition, Database, HotkeyAction, HotkeyConflict, NotificationTimeoutSeconds,
};
use crate::settings::SettingsStore;

use super::clipboard_listener::InternalClipboardMutationGuard;
use super::hotkey_capture::{
    HotkeyCaptureEffect, HotkeyCaptureHandle, HotkeyCaptureState, format_hotkey,
};
use super::keyboard_runtime::{RuntimeError, inject_ctrl_chord};
use super::selected_text_runtime::{set_image_clipboard_payload, set_unicode_clipboard};

const WINDOW_WIDTH: f32 = 520.0;
const WINDOW_HEIGHT: f32 = 420.0;
const SETTINGS_WIDTH: f32 = 540.0;
const SETTINGS_HEIGHT: f32 = 500.0;
const HOTKEY_CAPTURE_POLL_INTERVAL: Duration = Duration::from_millis(16);
const PREVIEW_WIDTH: f32 = WINDOW_WIDTH;
const PREVIEW_HEIGHT: f32 = WINDOW_HEIGHT;
const PREVIEW_GAP: f32 = 8.0;
const PREVIEW_HIDE_DELAY: Duration = Duration::from_secs(1);
const PREVIEW_SHOW_DELAY: Duration = Duration::from_millis(500);
const PREVIEW_KEEPALIVE_INTERVAL: Duration = Duration::from_millis(750);
const PARKED_POSITION: f32 = -32_000.0;
const ENTRY_LIMIT: usize = 100;
const INSERT_FOCUS_DELAY: Duration = Duration::from_millis(45);
// Clipboard history refresh is event-driven through ClipboardManagerState::serial.
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
const SCROLLBAR_IDLE: Color32 = Color32::from_rgb(82, 72, 48);
const SELECTED_TAB: Color32 = Color32::from_rgb(77, 63, 28);
const INACTIVE_TAB: Color32 = Color32::from_rgb(42, 43, 40);
const CLIPBOARD_WINDOW_TITLE: &str = "SunSwitcher Clipboard";
const SETTINGS_WINDOW_TITLE: &str = "SunSwitcher settings";
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
        let previous = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        let next = ClipboardManagerState {
            visible: true,
            active_tab,
            target_window_id,
            serial: previous.serial.wrapping_add(1),
        };
        let next_serial = next.serial;
        let state_changed = replace_clipboard_manager_state(&self.state, next);
        crate::runtime_log!(
            "[ui-command] clipboard show requested tab={:?} target_window_id={} previous_visible={} previous_serial={} next_serial={} state_changed={}",
            active_tab,
            target_window_id,
            previous.visible,
            previous.serial,
            next_serial,
            state_changed
        );
        self.repaint_ctx
            .send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        self.repaint_ctx
            .send_viewport_cmd(egui::ViewportCommand::Visible(true));
        self.repaint_ctx
            .send_viewport_cmd(egui::ViewportCommand::Focus);
        self.repaint_ctx.request_repaint();
        crate::runtime_log!("[ui-command] clipboard show viewport commands sent");
    }

    pub fn notify_history_changed(&self) {
        match self.state.write() {
            Ok(mut state) => {
                let previous_serial = state.serial;
                state.serial = state.serial.wrapping_add(1);
                crate::runtime_log!(
                    "[ui-command] clipboard history changed visible={} serial={} next_serial={}",
                    state.visible,
                    previous_serial,
                    state.serial
                );
            }
            Err(_) => {
                crate::runtime_log!("[ui-command] clipboard history changed but state lock failed")
            }
        }
        self.repaint_ctx.request_repaint();
    }
}

#[derive(Debug)]
struct SettingsUiState {
    visible: bool,
    title_bar_styled: bool,
    error: Option<String>,
    notification_timeout_edit: u32,
    notification_timeout_dirty: bool,
    capture_keyboard_state: [bool; 256],
}

#[derive(Clone)]
struct SettingsView {
    database_path: PathBuf,
    settings: SettingsStore,
    hotkey_capture: HotkeyCaptureHandle,
    state: Arc<Mutex<SettingsUiState>>,
}

#[derive(Debug, Clone, Copy)]
enum SettingsButtonIcon {
    Refresh,
    Confirm,
    Cancel,
}
pub(crate) struct ClipboardManagerApp {
    database_path: PathBuf,
    state: Arc<RwLock<ClipboardManagerState>>,
    shown: bool,
    filter: String,
    settings_view: SettingsView,
    last_serial: u64,
    preview: Arc<RwLock<Option<ClipboardPreview>>>,
    preview_hovered_at: Arc<RwLock<Option<Instant>>>,
    preview_candidate: Option<ClipboardPreviewCandidate>,
    preview_shown: bool,
    style_applied: bool,
    title_bar_styled: bool,
    entries_cache: Option<ClipboardEntriesCache>,
    dragging: Option<ClipboardDrag>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryActivation {
    CopyOnly,
    InsertIntoTarget,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardEntryMenuAction {
    Pin,
    Unpin,
    Clear,
    ClearAll,
}

#[derive(Debug, Clone)]
struct ClipboardPreview {
    entry: ClipboardEntryView,
    last_hovered: Instant,
}

#[derive(Debug, Clone)]
struct ClipboardPreviewCandidate {
    entry: ClipboardEntryView,
    requested_at: Instant,
}

#[derive(Debug, Clone)]
struct ClipboardEntriesCache {
    active_tab: ClipboardManagerTab,
    filter: String,
    result: Result<Arc<Vec<ClipboardEntryView>>, String>,
}

impl ClipboardEntriesCache {
    fn matches(&self, active_tab: ClipboardManagerTab, filter: &str) -> bool {
        self.active_tab == active_tab && self.filter == filter
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipboardDrag {
    entry_id: ClipboardEntryId,
    active_tab: ClipboardManagerTab,
}
impl ClipboardManagerApp {
    pub(crate) fn new(
        database_path: PathBuf,
        state: Arc<RwLock<ClipboardManagerState>>,
        settings: SettingsStore,
        hotkey_capture: HotkeyCaptureHandle,
    ) -> Self {
        let settings_view = SettingsView::new(database_path.clone(), settings, hotkey_capture);
        Self {
            database_path,
            state,
            shown: false,
            filter: String::new(),
            settings_view,
            last_serial: 0,
            preview: Arc::new(RwLock::new(None)),
            preview_hovered_at: Arc::new(RwLock::new(None)),
            preview_candidate: None,
            preview_shown: false,
            style_applied: false,
            title_bar_styled: false,
            entries_cache: None,
            dragging: None,
        }
    }
    pub(crate) fn logic(&mut self, ctx: &egui::Context) -> bool {
        self.settings_view.render_viewport(ctx);
        if ctx.input(|input| input.viewport().close_requested()) {
            // The root viewport is SunSwitcher's permanent eframe event pump. Native close
            // requests may be delivered again after the clipboard window was already parked;
            // accepting any of them would terminate the UI worker and strand both clipboard
            // and autocomplete state without a renderer.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            if self.shown {
                self.hide_shared(ctx);
            } else {
                crate::runtime_log!(
                    "[ui-command] root close canceled while clipboard already hidden"
                );
            }
            return false;
        }

        if self.shown && ctx.input(|input| input.viewport().minimized == Some(true)) {
            crate::runtime_log!("[ui-command] clipboard minimize requested; parking root viewport");
            self.hide_shared(ctx);
            return false;
        }

        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if state.serial != self.last_serial {
            let previous_serial = self.last_serial;
            self.last_serial = state.serial;
            self.invalidate_entries_cache();
            crate::runtime_log!(
                "[ui-frame] clipboard state observed previous_serial={} current_serial={} visible={} shown={} tab={:?} target_window_id={}",
                previous_serial,
                state.serial,
                state.visible,
                self.shown,
                state.active_tab,
                state.target_window_id
            );
            ctx.request_repaint();
        }
        self.set_visible(ctx, state.visible);
        if state.visible {
            if !self.style_applied {
                self.apply_style(ctx);
                self.style_applied = true;
            }
            if !self.title_bar_styled {
                self.title_bar_styled = apply_window_title_bar_color(CLIPBOARD_WINDOW_TITLE);
                if !self.title_bar_styled {
                    ctx.request_repaint_after(Duration::from_millis(50));
                }
            }
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
        // Style is applied once from logic() instead of every repaint.
        if ui.input(|input| input.key_pressed(egui::Key::Escape)) {
            self.hide_shared(ui.ctx());
            return true;
        }
        if let Some(tab) = self.consume_tab_switch_shortcuts(ui.ctx()) {
            self.set_active_tab(tab);
        }
        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        let entries = match self.load_entries_cached(state.active_tab) {
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

        let mut preview_source_hovered = false;
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
                                    self.preview_candidate = None;
                                    ui.colored_label(MUTED_TEXT, "Clipboard history is empty");
                                } else {
                                    let mut preview_candidate_touched = false;
                                    for (index, entry) in entries.iter().enumerate() {
                                        preview_candidate_touched |= self.entry_row(
                                            ui,
                                            state.active_tab,
                                            index,
                                            entry,
                                            state.target_window_id,
                                        );
                                    }
                                    if !preview_candidate_touched {
                                        self.preview_candidate = None;
                                    }
                                    preview_source_hovered = preview_candidate_touched;
                                    if ui.input(|input| input.pointer.any_released()) {
                                        self.dragging = None;
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
                        self.invalidate_entries_cache();
                        ui.ctx().request_repaint();
                    }
                    if gear_button(ui).clicked() {
                        self.settings_view.open(ui.ctx());
                    }
                });
            });
        self.expire_preview(ui.ctx(), preview_source_hovered);
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

    fn expire_preview(&mut self, ctx: &egui::Context, source_hovered: bool) {
        if preview_should_expire(self.preview_last_activity(), source_hovered, Instant::now()) {
            self.clear_preview(ctx);
        }
    }

    fn queue_preview_entry(&mut self, ctx: &egui::Context, entry: ClipboardEntryView) {
        let now = Instant::now();
        let mut promote = None;
        match self.preview_candidate.as_mut() {
            Some(candidate) if candidate.entry.id() == entry.id() => {
                let elapsed = now.duration_since(candidate.requested_at);
                if elapsed >= PREVIEW_SHOW_DELAY {
                    promote = Some(candidate.entry.clone());
                } else {
                    ctx.request_repaint_after(PREVIEW_SHOW_DELAY - elapsed);
                }
            }
            _ => {
                let current_preview_id = self
                    .preview
                    .read()
                    .ok()
                    .and_then(|preview| preview.as_ref().map(|preview| preview.entry.id()));
                if current_preview_id.is_some_and(|id| id != entry.id()) {
                    if let Ok(mut preview) = self.preview.write() {
                        *preview = None;
                    }
                    self.hide_preview_viewport(ctx);
                }
                self.preview_candidate = Some(ClipboardPreviewCandidate {
                    entry,
                    requested_at: now,
                });
                ctx.request_repaint_after(PREVIEW_SHOW_DELAY);
            }
        }
        if let Some(entry) = promote {
            self.set_preview_entry(ctx, entry);
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
        ctx.request_repaint_after(PREVIEW_KEEPALIVE_INTERVAL);
    }

    fn clear_preview(&mut self, ctx: &egui::Context) {
        self.preview_candidate = None;
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
        active_tab: ClipboardManagerTab,
        index: usize,
        entry: &ClipboardEntryView,
        target_window_id: usize,
    ) -> bool {
        let mut activation = None;
        let mut menu_action = None;
        let mut preview_candidate_touched = false;
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
            let is_dragging = self
                .dragging
                .is_some_and(|drag| drag.active_tab == active_tab && drag.entry_id == entry.id());
            let (response, text_truncated) = entry_text_response(ui, &summary, is_dragging);
            if response.drag_started() {
                self.dragging = Some(ClipboardDrag {
                    entry_id: entry.id(),
                    active_tab,
                });
                self.clear_preview(ui.ctx());
                ui.ctx().request_repaint();
            }
            if let Some(drag) = self.dragging
                && drag.active_tab == active_tab
                && drag.entry_id != entry.id()
                && response.hovered()
                && ui.input(|input| input.pointer.any_released())
            {
                let position = ui
                    .input(|input| input.pointer.interact_pos())
                    .map(|pointer| {
                        if pointer.y <= response.rect.center().y {
                            ClipboardReorderPosition::Before
                        } else {
                            ClipboardReorderPosition::After
                        }
                    })
                    .unwrap_or(ClipboardReorderPosition::After);
                self.reorder_entry(active_tab, drag.entry_id, entry.id(), position);
                self.dragging = None;
                ui.ctx().request_repaint();
            }
            if response.hovered() && self.dragging.is_none() {
                if entry_has_preview(entry, text_truncated) {
                    self.queue_preview_entry(ui.ctx(), entry.clone());
                    preview_candidate_touched = true;
                } else {
                    self.clear_preview(ui.ctx());
                }
            }
            let clicked = response.clicked() && self.dragging.is_none();
            let pinned = entry.pinned_at_ms().is_some();
            let pin_label = if pinned { "Unpin" } else { "Pin" };
            response.context_menu(|ui| {
                if ui.button(pin_label).clicked() {
                    menu_action = Some(if pinned {
                        ClipboardEntryMenuAction::Unpin
                    } else {
                        ClipboardEntryMenuAction::Pin
                    });
                    ui.close();
                }
                if ui.button("Clear").clicked() {
                    menu_action = Some(ClipboardEntryMenuAction::Clear);
                    ui.close();
                }
                if active_tab == ClipboardManagerTab::Current && ui.button("Clear All").clicked() {
                    menu_action = Some(ClipboardEntryMenuAction::ClearAll);
                    ui.close();
                }
            });
            if clicked {
                activation = Some(EntryActivation::CopyOnly);
            }
        });

        if let Some(action) = menu_action {
            match action {
                ClipboardEntryMenuAction::Pin => self.pin_entry(entry.id(), true),
                ClipboardEntryMenuAction::Unpin => self.pin_entry(entry.id(), false),
                ClipboardEntryMenuAction::Clear => {
                    self.clear_entry(entry.id());
                    self.clear_preview(ui.ctx());
                }
                ClipboardEntryMenuAction::ClearAll => {
                    self.clear_all_entries();
                    self.clear_preview(ui.ctx());
                }
            }
            self.invalidate_entries_cache();
            ui.ctx().request_repaint();
        }
        if let Some(activation) = activation {
            self.activate_entry(ui.ctx(), entry, target_window_id, activation);
        }
        preview_candidate_touched
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
            // Solid egui scrollbars take their idle handle color from inactive.bg_fill.
            // Keep weak widget backgrounds dark while making the idle thumb visible.
            style.visuals.widgets.inactive.bg_fill = SCROLLBAR_IDLE;
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
        crate::runtime_log!(
            "[ui-viewport] clipboard set_visible requested visible={} previous_shown={}",
            visible,
            self.shown
        );
        if visible {
            self.title_bar_styled = false;
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
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            crate::runtime_log!("[ui-viewport] clipboard visible/focus commands sent");
        } else {
            self.title_bar_styled = false;
            self.entries_cache = None;
            self.dragging = None;
            park_hidden_root_viewport(ctx);
            crate::runtime_log!("[ui-viewport] clipboard parked hidden root viewport");
        }
        self.shown = visible;
        crate::runtime_log!(
            "[ui-viewport] clipboard shown state updated shown={}",
            self.shown
        );
    }

    fn hide_shared(&mut self, ctx: &egui::Context) {
        crate::runtime_log!("[ui-command] clipboard hide requested shown={}", self.shown);
        if let Ok(mut state) = self.state.write() {
            state.visible = false;
            state.serial = state.serial.wrapping_add(1);
            crate::runtime_log!("[ui-command] clipboard hide state serial={}", state.serial);
        }
        if let Ok(mut preview) = self.preview.write() {
            *preview = None;
        }
        self.hide_preview_viewport(ctx);
        self.set_visible(ctx, false);
        ctx.request_repaint();
    }

    fn load_entries_cached(
        &mut self,
        active_tab: ClipboardManagerTab,
    ) -> Result<Arc<Vec<ClipboardEntryView>>, String> {
        let cache_valid = self
            .entries_cache
            .as_ref()
            .is_some_and(|cache| cache.matches(active_tab, &self.filter));
        if !cache_valid {
            let result = self.load_entries(active_tab).map(Arc::new);
            self.entries_cache = Some(ClipboardEntriesCache {
                active_tab,
                filter: self.filter.clone(),
                result,
            });
        }
        match &self
            .entries_cache
            .as_ref()
            .expect("cache was just filled")
            .result
        {
            Ok(entries) => Ok(Arc::clone(entries)),
            Err(error) => Err(error.clone()),
        }
    }

    fn invalidate_entries_cache(&mut self) {
        self.entries_cache = None;
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
            crate::runtime_log!("clipboard pin update skipped: {error}");
        }
    }

    fn clear_entry(&self, entry_id: ClipboardEntryId) {
        let Ok(database) = Database::open(&self.database_path) else {
            return;
        };
        match database.delete_clipboard_entry(entry_id) {
            Ok(true) => {}
            Ok(false) => crate::runtime_log!(
                "clipboard clear skipped: entry {} no longer exists",
                entry_id.get()
            ),
            Err(error) => crate::runtime_log!("clipboard clear failed: {error}"),
        }
    }

    fn clear_all_entries(&self) {
        let Ok(database) = Database::open(&self.database_path) else {
            return;
        };
        match database.clear_clipboard_history() {
            Ok(deleted) => crate::runtime_log!("clipboard clear all removed {deleted} entries"),
            Err(error) => crate::runtime_log!("clipboard clear all failed: {error}"),
        }
    }

    fn reorder_entry(
        &mut self,
        active_tab: ClipboardManagerTab,
        moved_entry_id: ClipboardEntryId,
        target_entry_id: ClipboardEntryId,
        position: ClipboardReorderPosition,
    ) {
        let Ok(mut database) = Database::open(&self.database_path) else {
            return;
        };
        let list = match active_tab {
            ClipboardManagerTab::Current => ClipboardEntryList::Current,
            ClipboardManagerTab::Pinned => ClipboardEntryList::Pinned,
        };
        if let Err(error) =
            database.reorder_clipboard_entry(list, moved_entry_id, target_entry_id, position)
        {
            crate::runtime_log!("clipboard reorder skipped: {error}");
        }
        self.invalidate_entries_cache();
    }

    fn mark_entry_used(&self, entry_id: ClipboardEntryId) {
        if let Ok(database) = Database::open(&self.database_path)
            && let Err(error) = database.mark_clipboard_entry_used(entry_id, now_ms())
        {
            crate::runtime_log!("clipboard entry usage update skipped: {error}");
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
                    crate::runtime_log!("clipboard text copy failed: {error}");
                    return;
                }
                self.hide_shared(ctx);
                if activation == EntryActivation::InsertIntoTarget {
                    if target_window_id == 0 {
                        crate::runtime_log!("clipboard text insertion skipped: no target window");
                        return;
                    }
                    let focused = unsafe { SetForegroundWindow(target_window_id as HWND) } != 0;
                    if !focused {
                        crate::runtime_log!(
                            "clipboard text insertion skipped: target window could not be focused"
                        );
                        return;
                    }
                    thread::sleep(INSERT_FOCUS_DELAY);
                    if let Err(error) = insert_clipboard_text() {
                        crate::runtime_log!("clipboard text insertion failed: {error:?}");
                    }
                }
            }
            ClipboardEntryContent::Image { format, data } => {
                if let Err(error) = copy_clipboard_image(format, data) {
                    crate::runtime_log!("clipboard image copy failed: {error}");
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
            // Preview inherits the manager style; avoid resetting style every preview frame.
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
                ui.ctx().request_repaint_after(PREVIEW_KEEPALIVE_INTERVAL);
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
        // Keepalive repaint is scheduled only when preview hover or entry state changes.
    }

    fn hide_preview_viewport(&mut self, ctx: &egui::Context) {
        if self.preview_shown {
            ctx.send_viewport_cmd_to(preview_viewport_id(), egui::ViewportCommand::Visible(false));
            self.preview_shown = false;
        }
    }
}

impl SettingsView {
    fn new(
        database_path: PathBuf,
        settings: SettingsStore,
        hotkey_capture: HotkeyCaptureHandle,
    ) -> Self {
        let notification_timeout_edit = settings
            .load()
            .map(|settings| settings.notification_timeout_seconds().get())
            .unwrap_or(NotificationTimeoutSeconds::DEFAULT.get());
        Self {
            database_path,
            settings,
            hotkey_capture,
            state: Arc::new(Mutex::new(SettingsUiState {
                visible: false,
                title_bar_styled: false,
                error: None,
                notification_timeout_edit,
                notification_timeout_dirty: false,
                capture_keyboard_state: keyboard_down_snapshot(),
            })),
        }
    }

    fn viewport_id() -> ViewportId {
        ViewportId::from_hash_of("sunswitcher_settings_child")
    }

    fn open(&self, ctx: &egui::Context) {
        self.hotkey_capture.cancel();
        if let Ok(mut state) = self.state.lock() {
            state.visible = true;
            state.title_bar_styled = false;
            state.error = None;
            state.capture_keyboard_state = keyboard_down_snapshot();
            if let Ok(settings) = self.settings.load() {
                state.notification_timeout_edit = settings.notification_timeout_seconds().get();
                state.notification_timeout_dirty = false;
            }
        }
        ctx.send_viewport_cmd_to(Self::viewport_id(), egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd_to(Self::viewport_id(), egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd_to(Self::viewport_id(), egui::ViewportCommand::Focus);
        ctx.request_repaint();
    }

    fn close(&self) {
        self.hotkey_capture.cancel();
        if let Ok(mut state) = self.state.lock() {
            state.visible = false;
            state.title_bar_styled = false;
        }
    }

    fn is_visible(&self) -> bool {
        self.state.lock().is_ok_and(|state| state.visible)
    }

    fn render_viewport(&self, ctx: &egui::Context) {
        if !self.is_visible() {
            return;
        }
        if matches!(
            self.hotkey_capture.state(),
            HotkeyCaptureState::Listening(_)
        ) {
            ctx.request_repaint_after(HOTKEY_CAPTURE_POLL_INTERVAL);
        }
        let view = self.clone();
        let viewport = ViewportBuilder::default()
            .with_title(SETTINGS_WINDOW_TITLE)
            .with_inner_size(vec2(SETTINGS_WIDTH, SETTINGS_HEIGHT))
            .with_min_inner_size(vec2(380.0, 300.0))
            .with_resizable(true)
            .with_decorations(true)
            .with_taskbar(true)
            .with_always_on_top()
            .with_visible(true);
        ctx.show_viewport_deferred(Self::viewport_id(), viewport, move |ui, _class| {
            if matches!(
                view.hotkey_capture.state(),
                HotkeyCaptureState::Listening(_)
            ) {
                ui.ctx().request_repaint_after(HOTKEY_CAPTURE_POLL_INTERVAL);
            }
            if ui.input(|input| input.viewport().close_requested()) {
                view.close();
                return;
            }
            if !view.hotkey_capture.is_active()
                && ui.input(|input| input.key_pressed(egui::Key::Escape))
            {
                view.close();
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }
            egui::Frame::new()
                .fill(WINDOW_BG)
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| view.contents(ui));
                });
        });
        self.ensure_title_bar_style(ctx);
    }

    fn ensure_title_bar_style(&self, ctx: &egui::Context) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.title_bar_styled {
            return;
        }
        state.title_bar_styled = apply_window_title_bar_color(SETTINGS_WINDOW_TITLE);
        if !state.title_bar_styled {
            ctx.request_repaint_after(Duration::from_millis(50));
        }
    }

    fn poll_hotkey_capture(&self, state: &mut SettingsUiState) {
        let current = keyboard_down_snapshot();
        if matches!(
            self.hotkey_capture.state(),
            HotkeyCaptureState::Listening(_)
        ) && let Some(vk_code) = first_new_capture_key(&state.capture_keyboard_state, &current)
        {
            let (control, shift, alt, win) = capture_modifiers(&current);
            let effect =
                self.hotkey_capture
                    .observe_key_down(u32::from(vk_code), control, shift, alt, win);
            if effect == HotkeyCaptureEffect::Captured {
                crate::runtime_log!(
                    "[hotkey-capture] settings fallback captured vk=0x{vk_code:02X} ctrl={control} shift={shift} alt={alt} win={win}"
                );
            }
        }
        state.capture_keyboard_state = current;
    }

    fn persist_settings(&self, state: &mut SettingsUiState, next: AppSettings) -> bool {
        match Database::open(&self.database_path).and_then(|database| database.save_settings(next))
        {
            Ok(()) => match self.settings.replace(next) {
                Ok(()) => {
                    state.error = None;
                    true
                }
                Err(error) => {
                    state.error = Some(error.to_string());
                    false
                }
            },
            Err(error) => {
                state.error = Some(error.to_string());
                false
            }
        }
    }

    fn hotkey_action_label(action: HotkeyAction) -> &'static str {
        match action {
            HotkeyAction::ClipboardCurrent => "Clipboard Current",
            HotkeyAction::ClipboardPinned => "Clipboard Pinned",
            HotkeyAction::CompletionAccept => "Completion accept",
            HotkeyAction::CompletionNextWord => "Completion next word",
            HotkeyAction::UndoOrForgetWord => "Undo / forget word",
            HotkeyAction::IgnoreWord => "Ignore word",
        }
    }

    fn hotkey_row(
        &self,
        ui: &mut egui::Ui,
        state: &mut SettingsUiState,
        current: &mut AppSettings,
        action: HotkeyAction,
    ) {
        let capture = self.hotkey_capture.state();
        ui.horizontal(|ui| {
            ui.add_sized(
                [160.0, 22.0],
                egui::Label::new(Self::hotkey_action_label(action)),
            );
            match capture {
                HotkeyCaptureState::Listening(capturing) if capturing == action => {
                    ui.add_sized(
                        [190.0, 22.0],
                        egui::Label::new(RichText::new("Press shortcut...").monospace()),
                    );
                    if settings_icon_button(ui, SettingsButtonIcon::Cancel, true, "Cancel")
                        .clicked()
                    {
                        self.hotkey_capture.cancel();
                    }
                }
                HotkeyCaptureState::Pending {
                    action: capturing,
                    binding,
                } if capturing == action => {
                    ui.add_sized(
                        [190.0, 22.0],
                        egui::Label::new(RichText::new(format_hotkey(binding)).monospace()),
                    );
                    if settings_icon_button(ui, SettingsButtonIcon::Confirm, true, "Save shortcut")
                        .clicked()
                    {
                        if let Some(conflict) = current.hotkey_conflict(action, binding) {
                            state.error = Some(match conflict {
                                HotkeyConflict::Configured(conflict) => format!(
                                    "Shortcut is already used by {}",
                                    Self::hotkey_action_label(conflict)
                                ),
                                HotkeyConflict::ReservedCompletionControl => {
                                    "Shortcut is reserved for autocomplete navigation".to_owned()
                                }
                            });
                        } else {
                            let next = current.with_hotkey(action, binding);
                            if self.persist_settings(state, next) {
                                *current = next;
                                self.hotkey_capture.cancel();
                            }
                        }
                    }
                    if settings_icon_button(ui, SettingsButtonIcon::Cancel, true, "Cancel")
                        .clicked()
                    {
                        self.hotkey_capture.cancel();
                    }
                }
                _ => {
                    ui.add_sized(
                        [190.0, 22.0],
                        egui::Label::new(
                            RichText::new(format_hotkey(current.hotkey(action))).monospace(),
                        ),
                    );
                    let can_start = matches!(capture, HotkeyCaptureState::Idle);
                    if settings_icon_button(
                        ui,
                        SettingsButtonIcon::Refresh,
                        can_start,
                        "Record a new shortcut",
                    )
                    .clicked()
                        && can_start
                    {
                        state.error = None;
                        state.capture_keyboard_state = keyboard_down_snapshot();
                        self.hotkey_capture.begin(action);
                    }
                }
            }
        });
    }

    fn contents(&self, ui: &mut egui::Ui) {
        let Ok(mut state) = self.state.lock() else {
            ui.colored_label(Color32::LIGHT_RED, "Settings state is unavailable");
            return;
        };
        self.poll_hotkey_capture(&mut state);
        let mut current = match self.settings.load() {
            Ok(settings) => settings,
            Err(error) => {
                ui.colored_label(Color32::LIGHT_RED, error.to_string());
                return;
            }
        };

        ui.heading("Hotkeys");
        ui.separator();
        for action in HotkeyAction::ALL {
            self.hotkey_row(ui, &mut state, &mut current, action);
        }
        ui.horizontal(|ui| {
            ui.add_sized([160.0, 22.0], egui::Label::new("Manual keyboard switch"));
            ui.add_sized(
                [190.0, 22.0],
                egui::Label::new(RichText::new("Double Shift").monospace()),
            );
        });

        ui.add_space(12.0);
        ui.heading("Features");
        ui.separator();
        let mut autocomplete = current.enable_autocomplete();
        if ui
            .checkbox(&mut autocomplete, "Enable autocomplete")
            .changed()
        {
            let next = current.with_enable_autocomplete(autocomplete);
            if self.persist_settings(&mut state, next) {
                current = next;
            }
        }
        let mut autocorrections = current.enable_autocorrections();
        if ui
            .checkbox(&mut autocorrections, "Enable autocorrections")
            .changed()
        {
            let next = current.with_enable_autocorrections(autocorrections);
            if self.persist_settings(&mut state, next) {
                current = next;
            }
        }
        let mut auto_switches = current.enable_auto_keyboard_switches();
        if ui
            .checkbox(&mut auto_switches, "Enable auto keyboard switches")
            .changed()
        {
            let next = current.with_enable_auto_keyboard_switches(auto_switches);
            if self.persist_settings(&mut state, next) {
                current = next;
            }
        }

        ui.add_space(12.0);
        ui.heading("Notifications");
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Notification timeout seconds");
            let response = ui.add(
                egui::DragValue::new(&mut state.notification_timeout_edit)
                    .range(NotificationTimeoutSeconds::MIN..=NotificationTimeoutSeconds::MAX),
            );
            if response.changed() {
                state.notification_timeout_dirty =
                    state.notification_timeout_edit != current.notification_timeout_seconds().get();
            }
            if state.notification_timeout_dirty
                && settings_icon_button(ui, SettingsButtonIcon::Confirm, true, "Save timeout")
                    .clicked()
            {
                match NotificationTimeoutSeconds::try_new(state.notification_timeout_edit) {
                    Ok(timeout) => {
                        let next = current.with_notification_timeout_seconds(timeout);
                        if self.persist_settings(&mut state, next) {
                            state.notification_timeout_dirty = false;
                        }
                    }
                    Err(error) => state.error = Some(error.to_string()),
                }
            }
        });

        if let Some(error) = &state.error {
            ui.add_space(8.0);
            ui.colored_label(Color32::LIGHT_RED, error);
        }
    }
}

fn settings_icon_button(
    ui: &mut egui::Ui,
    icon: SettingsButtonIcon,
    enabled: bool,
    hover_text: &'static str,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(vec2(24.0, 22.0), sense);
    let fill = if response.hovered() && enabled {
        ROW_HOVER_BG
    } else {
        PANEL_BG
    };
    let color = if enabled { ACCENT } else { MUTED_TEXT };
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 3.0, fill);
    painter.rect_stroke(
        rect,
        3.0,
        Stroke::new(1.0, ACCENT_DIM),
        egui::StrokeKind::Inside,
    );
    let center = rect.center();
    match icon {
        SettingsButtonIcon::Refresh => {
            let radius = 5.3;
            let stroke = Stroke::new(1.5, color);
            for (start, end) in [
                (std::f32::consts::PI * 1.08, std::f32::consts::PI * 1.92),
                (std::f32::consts::PI * 0.08, std::f32::consts::PI * 0.92),
            ] {
                let mut previous = center + vec2(start.cos(), start.sin()) * radius;
                for step in 1..=10 {
                    let angle = start + (end - start) * step as f32 / 10.0;
                    let next = center + vec2(angle.cos(), angle.sin()) * radius;
                    painter.line_segment([previous, next], stroke);
                    previous = next;
                }
                let tangent = vec2(-end.sin(), end.cos());
                let radial = vec2(end.cos(), end.sin());
                let head_base = previous - tangent * 3.0;
                painter.line_segment([previous, head_base + radial * 1.8], stroke);
                painter.line_segment([previous, head_base - radial * 1.8], stroke);
            }
        }
        SettingsButtonIcon::Confirm => {
            painter.line_segment(
                [center + vec2(-5.0, 0.0), center + vec2(-1.5, 3.5)],
                Stroke::new(1.8, color),
            );
            painter.line_segment(
                [center + vec2(-1.5, 3.5), center + vec2(5.5, -4.0)],
                Stroke::new(1.8, color),
            );
        }
        SettingsButtonIcon::Cancel => {
            painter.line_segment(
                [center + vec2(-4.0, -4.0), center + vec2(4.0, 4.0)],
                Stroke::new(1.6, color),
            );
            painter.line_segment(
                [center + vec2(-4.0, 4.0), center + vec2(4.0, -4.0)],
                Stroke::new(1.6, color),
            );
        }
    }
    response.on_hover_text(hover_text)
}

fn keyboard_down_snapshot() -> [bool; 256] {
    let mut down = [false; 256];
    for (vk_code, is_down) in down.iter_mut().enumerate().skip(0x08) {
        if vk_code == VK_PAUSE as usize {
            continue;
        }
        *is_down = unsafe { GetAsyncKeyState(vk_code as i32) < 0 };
    }
    down[VK_CANCEL as usize] = async_key_down_or_pressed(VK_CANCEL);
    down[VK_PAUSE as usize] = async_key_down_or_pressed(VK_PAUSE);
    down
}

fn async_key_down_or_pressed(vk_code: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(vk_code as i32) } as u16;
    state & 0x8001 != 0
}

fn first_new_capture_key(previous: &[bool; 256], current: &[bool; 256]) -> Option<u16> {
    [VK_CANCEL, VK_PAUSE]
        .into_iter()
        .find(|&vk_code| {
            let index = usize::from(vk_code);
            current[index] && !previous[index]
        })
        .or_else(|| {
            (0x08u16..=0xFE).find(|&vk_code| {
                let index = usize::from(vk_code);
                current[index] && !previous[index] && !is_capture_modifier_vk(vk_code)
            })
        })
}

fn is_capture_modifier_vk(vk_code: u16) -> bool {
    [
        VK_CONTROL,
        VK_LCONTROL,
        VK_RCONTROL,
        VK_SHIFT,
        VK_LSHIFT,
        VK_RSHIFT,
        VK_MENU,
        VK_LMENU,
        VK_RMENU,
        VK_LWIN,
        VK_RWIN,
    ]
    .contains(&vk_code)
}

fn capture_modifiers(down: &[bool; 256]) -> (bool, bool, bool, bool) {
    let any_down = |keys: &[u16]| keys.iter().any(|key| down[usize::from(*key)]);
    (
        any_down(&[VK_CONTROL, VK_LCONTROL, VK_RCONTROL]),
        any_down(&[VK_SHIFT, VK_LSHIFT, VK_RSHIFT]),
        any_down(&[VK_MENU, VK_LMENU, VK_RMENU]),
        any_down(&[VK_LWIN, VK_RWIN]),
    )
}
// Settings icon rendering and keyboard-capture helpers are defined above.
fn entry_text_response(ui: &mut egui::Ui, text: &str, dragging: bool) -> (egui::Response, bool) {
    let width = ui.available_width().max(80.0);
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, ENTRY_ROW_HEIGHT), Sense::click_and_drag());
    let fill = if dragging {
        SELECTED_TAB
    } else if response.hovered() {
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
    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
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

fn preview_should_expire(
    last_activity: Option<Instant>,
    source_hovered: bool,
    now: Instant,
) -> bool {
    !source_hovered
        && last_activity.is_some_and(|last_activity| {
            now.saturating_duration_since(last_activity) > PREVIEW_HIDE_DELAY
        })
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

// Native child viewports do not inherit the manager root caption styling.
fn apply_window_title_bar_color(title: &str) -> bool {
    let Some(hwnd) = find_current_process_window(title) else {
        return false;
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
    true
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

    fn test_app(state: Arc<RwLock<ClipboardManagerState>>) -> ClipboardManagerApp {
        let repaint_ctx = egui::Context::default();
        let settings = SettingsStore::new(AppSettings::default());
        let hotkey_capture =
            HotkeyCaptureHandle::new(HotkeyCaptureHandle::shared_state(), repaint_ctx);
        ClipboardManagerApp::new(std::path::PathBuf::new(), state, settings, hotkey_capture)
    }

    #[test]
    fn settings_hotkey_fallback_detects_new_key_with_current_modifiers() {
        let previous = [false; 256];
        let mut current = previous;
        current[usize::from(VK_CONTROL)] = true;
        current[usize::from(VK_SHIFT)] = true;
        current[0xBD] = true;

        assert_eq!(first_new_capture_key(&previous, &current), Some(0xBD));
        assert_eq!(capture_modifiers(&current), (true, true, false, false));
    }

    #[test]
    fn settings_ctrl_pause_capture_prefers_cancel_over_other_reported_keys() {
        let previous = [false; 256];
        let mut current = previous;
        current[usize::from(VK_CONTROL)] = true;
        current[usize::from(VK_CANCEL)] = true;
        current[0xC0] = true;

        assert_eq!(first_new_capture_key(&previous, &current), Some(VK_CANCEL));
        assert_eq!(capture_modifiers(&current), (true, false, false, false));
    }

    #[test]
    fn settings_hotkey_fallback_ignores_modifier_only_and_already_down_keys() {
        let mut previous = [false; 256];
        previous[0xBB] = true;
        let mut current = previous;
        current[usize::from(VK_CONTROL)] = true;

        assert_eq!(first_new_capture_key(&previous, &current), None);
    }

    #[test]
    fn preview_expiry_respects_live_source_hover() {
        let now = Instant::now();
        let stale = now - PREVIEW_HIDE_DELAY - Duration::from_millis(1);

        assert!(!preview_should_expire(Some(stale), true, now));
        assert!(preview_should_expire(Some(stale), false, now));
        assert!(!preview_should_expire(None, false, now));
    }

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
    fn entry_cache_validity_is_event_driven_by_tab_and_filter() {
        let cache = ClipboardEntriesCache {
            active_tab: ClipboardManagerTab::Current,
            filter: "needle".to_owned(),
            result: Ok(Arc::new(Vec::new())),
        };
        assert!(cache.matches(ClipboardManagerTab::Current, "needle"));
        assert!(!cache.matches(ClipboardManagerTab::Pinned, "needle"));
        assert!(!cache.matches(ClipboardManagerTab::Current, "other"));
    }

    #[test]
    fn repeated_root_close_is_canceled_after_clipboard_window_is_parked() {
        let state = Arc::new(RwLock::new(ClipboardManagerState {
            visible: true,
            active_tab: ClipboardManagerTab::Current,
            target_window_id: 42,
            serial: 1,
        }));
        let mut app = test_app(Arc::clone(&state));
        app.shown = true;
        let ctx = egui::Context::default();

        let mut first_input = egui::RawInput::default();
        first_input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .events
            .push(egui::ViewportEvent::Close);
        let first_output = ctx.run_logic(&first_input, |ctx| {
            assert!(!app.logic(ctx));
        });
        assert!(
            first_output
                .viewport_commands
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|commands| commands.contains(&egui::ViewportCommand::CancelClose))
        );
        assert!(!app.shown);
        assert!(!state.read().expect("clipboard state").visible);

        let mut repeated_input = egui::RawInput::default();
        repeated_input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .events
            .push(egui::ViewportEvent::Close);
        let repeated_output = ctx.run_logic(&repeated_input, |ctx| {
            assert!(!app.logic(ctx));
        });
        assert!(
            repeated_output
                .viewport_commands
                .get(&egui::ViewportId::ROOT)
                .is_some_and(|commands| commands.contains(&egui::ViewportCommand::CancelClose))
        );
    }

    #[test]
    fn show_command_restores_a_minimized_root() {
        let state = Arc::new(RwLock::new(ClipboardManagerState::default()));
        let ctx = egui::Context::default();
        let output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            let handle = ClipboardManagerHandle::new(Arc::clone(&state), ctx.clone());
            handle.show(ClipboardManagerTab::Current, 42);
        });
        let commands = output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .expect("root viewport show commands");
        assert!(commands.contains(&egui::ViewportCommand::Minimized(false)));
        assert!(commands.contains(&egui::ViewportCommand::Visible(true)));
        assert!(commands.contains(&egui::ViewportCommand::Focus));
        let shared = state.read().expect("clipboard state");
        assert!(shared.visible);
        assert_eq!(shared.active_tab, ClipboardManagerTab::Current);
        assert_eq!(shared.target_window_id, 42);
    }

    #[test]
    fn minimizing_clipboard_parks_root_and_reopen_unminimizes() {
        let state = Arc::new(RwLock::new(ClipboardManagerState {
            visible: true,
            active_tab: ClipboardManagerTab::Current,
            target_window_id: 42,
            serial: 1,
        }));
        let mut app = test_app(Arc::clone(&state));
        app.shown = true;
        app.last_serial = 1;
        let ctx = egui::Context::default();

        let mut minimized_input = egui::RawInput::default();
        minimized_input
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .expect("root viewport")
            .minimized = Some(true);
        let minimized_output = ctx.run_logic(&minimized_input, |ctx| {
            assert!(!app.logic(ctx));
        });
        let minimized_commands = minimized_output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .expect("root viewport commands after minimize");
        assert!(minimized_commands.contains(&egui::ViewportCommand::Minimized(false)));
        assert!(minimized_commands.contains(&egui::ViewportCommand::Visible(true)));
        assert!(!app.shown);
        assert!(!state.read().expect("clipboard state").visible);

        {
            let mut shared = state.write().expect("clipboard state");
            shared.visible = true;
            shared.serial = shared.serial.wrapping_add(1);
        }
        let reopened_output = ctx.run_logic(&egui::RawInput::default(), |ctx| {
            assert!(app.logic(ctx));
        });
        let reopened_commands = reopened_output
            .viewport_commands
            .get(&egui::ViewportId::ROOT)
            .expect("root viewport commands after reopen");
        assert!(reopened_commands.contains(&egui::ViewportCommand::Minimized(false)));
        assert!(reopened_commands.contains(&egui::ViewportCommand::Visible(true)));
        assert!(reopened_commands.contains(&egui::ViewportCommand::Focus));
        assert!(app.shown);
    }

    #[test]
    fn clipboard_style_keeps_idle_scrollbar_handle_visible() {
        let state = Arc::new(RwLock::new(ClipboardManagerState::default()));
        let app = test_app(state);
        let ctx = egui::Context::default();
        app.apply_style(&ctx);

        for theme in [egui::Theme::Light, egui::Theme::Dark] {
            let style = ctx.style_of(theme);
            assert!(!style.spacing.scroll.floating);
            assert!(!style.spacing.scroll.foreground_color);
            assert_eq!(style.visuals.widgets.inactive.bg_fill, SCROLLBAR_IDLE);
            assert_ne!(style.visuals.widgets.inactive.bg_fill, ROW_BG);
        }
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
