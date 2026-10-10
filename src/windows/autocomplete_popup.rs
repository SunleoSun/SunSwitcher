use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use eframe::egui::{self, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2};

use crate::settings::SettingsStore;

use super::caret_locator::{CaretAnchor, CaretLocator, CaretSource, popup_placement};
use super::clipboard_manager::{
    ClipboardManagerApp, ClipboardManagerHandle, ClipboardManagerState, ClipboardManagerTab,
};
use super::hotkey_capture::HotkeyCaptureHandle;
use super::tray_icon::TrayIcon;

const POPUP_WIDTH: f32 = 360.0;
const MAX_POPUP_SUGGESTIONS: usize = 8;
const ROW_HEIGHT: f32 = 24.0;
const POPUP_PADDING: f32 = 12.0;
const CARET_GAP: f32 = 6.0;
const NOTIFICATION_WIDTH: f32 = 360.0;
const NOTIFICATION_HEIGHT: f32 = 48.0;
const NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(3);
const PARKED_POSITION: f32 = -32_000.0;
const SUGGESTION_BG: egui::Color32 = egui::Color32::from_rgb(30, 32, 38);
const SUGGESTION_SELECTED: egui::Color32 = egui::Color32::from_rgb(77, 63, 28);
const SUGGESTION_TEXT: egui::Color32 = egui::Color32::from_rgb(244, 242, 226);
const SUGGESTION_BORDER: egui::Color32 = egui::Color32::from_rgb(104, 84, 32);
const WINDOW_ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");

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

fn load_window_icon() -> Option<Arc<egui::IconData>> {
    let decoded = image::load_from_memory(WINDOW_ICON_PNG).ok()?;
    let rgba = decoded.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    Some(Arc::new(egui::IconData {
        rgba: rgba.into_raw(),
        width,
        height,
    }))
}

fn suggestion_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("sunswitcher_suggestions_child")
}

fn notification_viewport_id() -> ViewportId {
    ViewportId::from_hash_of("sunswitcher_notification_child")
}

fn render_suggestion_rows(ui: &mut egui::Ui, state: &PopupState) {
    let bounds = ui.max_rect();
    ui.painter().rect_filled(bounds, 6.0, SUGGESTION_BG);
    ui.painter().rect_stroke(
        bounds.shrink(0.5),
        6.0,
        egui::Stroke::new(1.0, SUGGESTION_BORDER),
        egui::StrokeKind::Inside,
    );

    let row_width = POPUP_WIDTH - POPUP_PADDING * 2.0;
    let origin = bounds.left_top() + vec2(POPUP_PADDING, POPUP_PADDING);
    for (index, suggestion) in state
        .suggestions
        .iter()
        .take(MAX_POPUP_SUGGESTIONS)
        .enumerate()
    {
        let row = egui::Rect::from_min_size(
            origin + vec2(0.0, ROW_HEIGHT * index as f32),
            vec2(row_width, ROW_HEIGHT),
        );
        if index == state.selected {
            ui.painter().rect_filled(row, 4.0, SUGGESTION_SELECTED);
            ui.painter().line_segment(
                [row.left_top(), row.left_bottom()],
                egui::Stroke::new(2.0, egui::Color32::from_rgb(226, 184, 63)),
            );
        }
        ui.painter().text(
            row.left_center() + vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            suggestion,
            egui::FontId::proportional(15.0),
            SUGGESTION_TEXT,
        );
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct PopupState {
    suggestions: Vec<String>,
    selected: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SuggestionViewportSpec {
    state: PopupState,
    prefer_root_anchor: bool,
    x: i32,
    y: i32,
    height: i32,
}

#[derive(Debug, Default)]
struct SuggestionViewportCache {
    visible: bool,
    spec: Option<SuggestionViewportSpec>,
}

#[derive(Debug, Clone)]
struct NotificationState {
    message: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NotificationViewportSpec {
    message: String,
    x: i32,
    y: i32,
}

#[derive(Debug, Default)]
struct NotificationViewportCache {
    visible: bool,
    spec: Option<NotificationViewportSpec>,
}

fn suggestion_notification_anchor(spec: &SuggestionViewportSpec) -> CaretAnchor {
    CaretAnchor::new(
        spec.x as f32,
        spec.y as f32,
        spec.height as f32,
        CaretSource::ForegroundWindowFallback,
    )
}

#[derive(Clone)]
pub struct TransientNotificationHandle {
    state: Arc<RwLock<Option<NotificationState>>>,
    repaint_ctx: egui::Context,
    settings: SettingsStore,
}

impl TransientNotificationHandle {
    pub fn show(&self, message: impl Into<String>) {
        let message = message.into();
        if message.is_empty() {
            return;
        }
        crate::runtime_log!("[notification] show message={message:?}");
        if let Ok(mut state) = self.state.write() {
            let timeout = self
                .settings
                .load()
                .map(|settings| settings.notification_timeout_seconds().duration())
                .unwrap_or(NOTIFICATION_TIMEOUT);
            *state = Some(NotificationState {
                message,
                expires_at: Instant::now() + timeout,
            });
        }
        self.repaint_ctx.request_repaint();
        self.repaint_ctx
            .request_repaint_of(notification_viewport_id());
    }
}

pub struct AutocompletePopupHandle {
    state: Arc<RwLock<PopupState>>,
    notification: TransientNotificationHandle,
    hotkey_capture: HotkeyCaptureHandle,
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
    pub fn start(database_path: PathBuf, settings: SettingsStore) -> Result<Self, String> {
        let state = Arc::new(RwLock::new(PopupState::default()));
        let notification_state = Arc::new(RwLock::new(None));
        let hotkey_capture_state = HotkeyCaptureHandle::shared_state();
        let clipboard_state = Arc::new(RwLock::new(ClipboardManagerState::default()));
        let shutdown = Arc::new(AtomicBool::new(false));
        let worker_state = Arc::clone(&state);
        let worker_notification_state = Arc::clone(&notification_state);
        let worker_hotkey_capture_state = Arc::clone(&hotkey_capture_state);
        let worker_clipboard_state = Arc::clone(&clipboard_state);
        let worker_shutdown = Arc::clone(&shutdown);
        let worker_settings = settings.clone();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            let mut viewport = ViewportBuilder::default()
                .with_title("SunSwitcher UI")
                .with_inner_size(vec2(1.0, 1.0))
                .with_position(pos2(PARKED_POSITION, PARKED_POSITION))
                .with_resizable(false)
                .with_decorations(false)
                .with_taskbar(false)
                .with_active(false)
                .with_visible(true)
                .with_always_on_top()
                .with_mouse_passthrough(false)
                .with_transparent(false);
            if let Some(icon) = load_window_icon() {
                viewport = viewport.with_icon(icon);
            }
            let options = eframe::NativeOptions {
                event_loop_builder: Some(Box::new(|builder| {
                    use winit::platform::windows::EventLoopBuilderExtWindows as _;
                    builder.with_any_thread(true);
                })),
                viewport,
                ..Default::default()
            };

            let startup_sender = ready_sender.clone();
            let result = eframe::run_native(
                "SunSwitcher UI",
                options,
                Box::new(move |creation_context| {
                    install_system_font(&creation_context.egui_ctx)?;
                    creation_context
                        .egui_ctx
                        .options_mut(|options| options.zoom_with_keyboard = false);
                    let repaint_ctx = creation_context.egui_ctx.clone();
                    let notification = TransientNotificationHandle {
                        state: worker_notification_state,
                        repaint_ctx: repaint_ctx.clone(),
                        settings: worker_settings.clone(),
                    };
                    let hotkey_capture =
                        HotkeyCaptureHandle::new(worker_hotkey_capture_state, repaint_ctx.clone());
                    let _ = startup_sender.send(Ok(repaint_ctx));
                    Ok(Box::new(AutocompletePopupApp {
                        state: worker_state,
                        notification,
                        settings: worker_settings.clone(),
                        clipboard: ClipboardManagerApp::new(
                            database_path,
                            worker_clipboard_state,
                            worker_settings,
                            hotkey_capture,
                        ),
                        shutdown: worker_shutdown,
                        locator: CaretLocator::new(),
                        fallback_anchor: None,
                        shown: false,
                        suggestion_viewport: SuggestionViewportCache::default(),
                        notification_viewport: NotificationViewportCache::default(),
                    }))
                }),
            );
            if let Err(error) = result {
                let _ = ready_sender.send(Err(error.to_string()));
            }
        });

        match ready_receiver.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(repaint_ctx)) => {
                crate::runtime_log!("[ui-start] SunSwitcher UI runtime initialized");
                let notification = TransientNotificationHandle {
                    state: notification_state,
                    repaint_ctx: repaint_ctx.clone(),
                    settings: settings.clone(),
                };
                let hotkey_capture =
                    HotkeyCaptureHandle::new(hotkey_capture_state, repaint_ctx.clone());
                let clipboard = ClipboardManagerHandle::new(clipboard_state, repaint_ctx.clone());
                let tray = TrayIcon::start(
                    clipboard.clone(),
                    Arc::clone(&shutdown),
                    repaint_ctx.clone(),
                );
                Ok(Self {
                    state,
                    notification,
                    hotkey_capture,
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
                crate::runtime_log!("[ui-start] SunSwitcher UI did not initialize: {error}");
                Err(format!("SunSwitcher UI did not initialize: {error}"))
            }
        }
    }

    pub fn update<'a>(&self, suggestions: impl IntoIterator<Item = &'a str>, selected: usize) {
        let suggestions = suggestions
            .into_iter()
            .take(MAX_POPUP_SUGGESTIONS)
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let next = PopupState {
            selected: selected.min(suggestions.len().saturating_sub(1)),
            suggestions,
        };
        let changed = replace_popup_state(&self.state, next);
        if changed {
            self.repaint_ctx.request_repaint();
            self.repaint_ctx
                .request_repaint_of(suggestion_viewport_id());
        }
    }

    pub fn hide(&self) {
        let changed = replace_popup_state(&self.state, PopupState::default());
        if changed {
            self.repaint_ctx.request_repaint();
            self.repaint_ctx
                .request_repaint_of(suggestion_viewport_id());
        }
    }

    pub fn show_clipboard(&self, active_tab: ClipboardManagerTab, target_window_id: usize) {
        crate::runtime_log!(
            "[ui-command] show_clipboard dispatch tab={:?} target_window_id={}",
            active_tab,
            target_window_id
        );
        self.clipboard.show(active_tab, target_window_id);
    }

    pub fn clipboard_handle(&self) -> ClipboardManagerHandle {
        self.clipboard.clone()
    }

    pub fn notification_handle(&self) -> TransientNotificationHandle {
        self.notification.clone()
    }

    pub fn hotkey_capture_handle(&self) -> HotkeyCaptureHandle {
        self.hotkey_capture.clone()
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
    notification: TransientNotificationHandle,
    settings: SettingsStore,
    clipboard: ClipboardManagerApp,
    shutdown: Arc<AtomicBool>,
    locator: CaretLocator,
    fallback_anchor: Option<CaretAnchor>,
    shown: bool,
    suggestion_viewport: SuggestionViewportCache,
    notification_viewport: NotificationViewportCache,
}

fn park_root_viewport(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Decorations(false));
    ctx.send_viewport_cmd(ViewportCommand::Resizable(false));
    ctx.send_viewport_cmd(ViewportCommand::InnerSize(vec2(1.0, 1.0)));
    ctx.send_viewport_cmd(ViewportCommand::OuterPosition(pos2(
        PARKED_POSITION,
        PARKED_POSITION,
    )));
    ctx.send_viewport_cmd(ViewportCommand::Visible(true));
}

fn root_viewport_anchor(ctx: &egui::Context) -> Option<CaretAnchor> {
    ctx.input(|input| {
        let viewport = input.viewport();
        let root = viewport.outer_rect.or(viewport.inner_rect)?;
        let x = root.left() + POPUP_PADDING;
        let y = (root.bottom() - POPUP_PADDING - ROW_HEIGHT).max(root.top() + POPUP_PADDING);
        Some(CaretAnchor::new(
            x,
            y,
            ROW_HEIGHT,
            CaretSource::ForegroundWindowFallback,
        ))
    })
}

impl AutocompletePopupApp {
    fn hide_suggestion_viewport(&mut self, ctx: &egui::Context) {
        if self.suggestion_viewport.visible {
            ctx.send_viewport_cmd_to(suggestion_viewport_id(), ViewportCommand::Visible(false));
        }
        self.suggestion_viewport.visible = false;
        self.suggestion_viewport.spec = None;
    }

    fn show_suggestion_viewport(&mut self, ctx: &egui::Context, prefer_root_anchor: bool) {
        if !self
            .settings
            .load()
            .is_ok_and(|settings| settings.enable_autocomplete())
        {
            self.hide_suggestion_viewport(ctx);
            return;
        }
        let state = self
            .state
            .read()
            .map(|state| state.clone())
            .unwrap_or_default();
        if state.suggestions.is_empty() {
            self.hide_suggestion_viewport(ctx);
            return;
        }

        if !prefer_root_anchor
            && let Some(spec) = self.suggestion_viewport.spec.clone().filter(|spec| {
                self.suggestion_viewport.visible && spec.state == state && !spec.prefer_root_anchor
            })
        {
            self.render_suggestion_viewport(ctx, &spec);
            return;
        }

        let anchor = if prefer_root_anchor {
            root_viewport_anchor(ctx).or_else(|| self.locator.fallback_anchor())
        } else if let Some(anchor) = self.locator.locate() {
            self.fallback_anchor = None;
            Some(anchor)
        } else {
            if self.fallback_anchor.is_none() {
                self.fallback_anchor = self.locator.fallback_anchor();
            }
            self.fallback_anchor
        };
        let Some(anchor) = anchor else {
            self.hide_suggestion_viewport(ctx);
            return;
        };

        let visible_rows = state.suggestions.len().min(MAX_POPUP_SUGGESTIONS);
        let popup_height = POPUP_PADDING * 2.0 + ROW_HEIGHT * visible_rows as f32;
        let placement = popup_placement(anchor, POPUP_WIDTH, popup_height, CARET_GAP);
        let spec = SuggestionViewportSpec {
            state,
            prefer_root_anchor,
            x: placement.x.round() as i32,
            y: placement.y.round() as i32,
            height: popup_height.round() as i32,
        };
        if self.suggestion_viewport.visible && self.suggestion_viewport.spec.as_ref() == Some(&spec)
        {
            self.render_suggestion_viewport(ctx, &spec);
            return;
        }

        self.render_suggestion_viewport(ctx, &spec);
        self.suggestion_viewport.visible = true;
        self.suggestion_viewport.spec = Some(spec);
    }

    fn render_suggestion_viewport(&self, ctx: &egui::Context, spec: &SuggestionViewportSpec) {
        let state_handle = Arc::clone(&self.state);
        let viewport = ViewportBuilder::default()
            .with_title("SunSwitcher suggestions")
            .with_inner_size(vec2(POPUP_WIDTH, spec.height as f32))
            .with_position(pos2(spec.x as f32, spec.y as f32))
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(false)
            .with_visible(true)
            .with_transparent(false);
        ctx.show_viewport_deferred(suggestion_viewport_id(), viewport, move |ui, _class| {
            let state = state_handle
                .read()
                .map(|state| state.clone())
                .unwrap_or_default();
            if !state.suggestions.is_empty() {
                render_suggestion_rows(ui, &state);
            }
        });
    }

    fn hide_notification_viewport(&mut self, ctx: &egui::Context) {
        if self.notification_viewport.visible {
            ctx.send_viewport_cmd_to(notification_viewport_id(), ViewportCommand::Visible(false));
        }
        self.notification_viewport.visible = false;
        self.notification_viewport.spec = None;
    }

    fn active_notification(&self) -> Option<NotificationState> {
        let now = Instant::now();
        let mut state = self.notification.state.write().ok()?;
        if state
            .as_ref()
            .is_some_and(|notification| notification.expires_at <= now)
        {
            *state = None;
            return None;
        }
        state.clone()
    }

    fn notification_anchor(
        &mut self,
        ctx: &egui::Context,
        prefer_root_anchor: bool,
    ) -> Option<CaretAnchor> {
        if prefer_root_anchor {
            return root_viewport_anchor(ctx).or_else(|| self.locator.fallback_anchor());
        }
        if self.suggestion_viewport.visible
            && let Some(spec) = self.suggestion_viewport.spec.as_ref()
        {
            return Some(suggestion_notification_anchor(spec));
        }
        if let Some(anchor) = self.locator.locate() {
            self.fallback_anchor = None;
            Some(anchor)
        } else {
            if self.fallback_anchor.is_none() {
                self.fallback_anchor = self.locator.fallback_anchor();
            }
            self.fallback_anchor
        }
    }

    fn show_notification_viewport(&mut self, ctx: &egui::Context, prefer_root_anchor: bool) {
        let Some(notification) = self.active_notification() else {
            self.hide_notification_viewport(ctx);
            return;
        };
        let remaining = notification
            .expires_at
            .saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            self.hide_notification_viewport(ctx);
            return;
        }
        ctx.request_repaint_after(remaining);

        let Some(anchor) = self.notification_anchor(ctx, prefer_root_anchor) else {
            self.hide_notification_viewport(ctx);
            return;
        };
        let placement = popup_placement(anchor, NOTIFICATION_WIDTH, NOTIFICATION_HEIGHT, CARET_GAP);
        let spec = NotificationViewportSpec {
            message: notification.message,
            x: placement.x.round() as i32,
            y: placement.y.round() as i32,
        };
        self.render_notification_viewport(ctx, &spec);
        self.notification_viewport.visible = true;
        self.notification_viewport.spec = Some(spec);
    }

    fn render_notification_viewport(&self, ctx: &egui::Context, spec: &NotificationViewportSpec) {
        let message = spec.message.clone();
        let viewport = ViewportBuilder::default()
            .with_title("SunSwitcher notification")
            .with_inner_size(vec2(NOTIFICATION_WIDTH, NOTIFICATION_HEIGHT))
            .with_position(pos2(spec.x as f32, spec.y as f32))
            .with_decorations(false)
            .with_resizable(false)
            .with_taskbar(false)
            .with_always_on_top()
            .with_active(false)
            .with_visible(true)
            .with_transparent(false);
        ctx.show_viewport_deferred(notification_viewport_id(), viewport, move |ui, _class| {
            let bounds = ui.max_rect();
            ui.painter().rect_filled(bounds, 6.0, SUGGESTION_BG);
            ui.painter().rect_stroke(
                bounds.shrink(0.5),
                6.0,
                egui::Stroke::new(1.0, SUGGESTION_BORDER),
                egui::StrokeKind::Inside,
            );
            ui.painter().text(
                bounds.left_center() + vec2(POPUP_PADDING, 0.0),
                egui::Align2::LEFT_CENTER,
                message.as_str(),
                egui::FontId::proportional(15.0),
                SUGGESTION_TEXT,
            );
        });
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
            self.shown = false;
            self.show_suggestion_viewport(ctx, true);
            self.show_notification_viewport(ctx, true);
            return;
        }

        let has_suggestions = self
            .state
            .read()
            .is_ok_and(|state| !state.suggestions.is_empty());
        if has_suggestions {
            if !self.shown {
                park_root_viewport(ctx);
                self.shown = true;
            }
            self.show_suggestion_viewport(ctx, false);
        } else {
            self.fallback_anchor = None;
            self.hide_suggestion_viewport(ctx);
        }
        self.show_notification_viewport(ctx, false);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.shutdown.load(Ordering::Acquire) {
            return;
        }
        if self.clipboard.ui(ui) {
            self.show_suggestion_viewport(ui.ctx(), true);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [
            f32::from(SUGGESTION_BG.r()) / 255.0,
            f32::from(SUGGESTION_BG.g()) / 255.0,
            f32::from(SUGGESTION_BG.b()) / 255.0,
            1.0,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notification_updates_replace_the_message_and_restart_the_deadline() {
        let state = Arc::new(RwLock::new(None));
        let handle = TransientNotificationHandle {
            state: Arc::clone(&state),
            repaint_ctx: egui::Context::default(),
            settings: SettingsStore::new(crate::persistence::AppSettings::default()),
        };

        handle.show("first");
        let first = state
            .read()
            .expect("notification state lock")
            .clone()
            .expect("first notification");
        handle.show("second");
        let second = state
            .read()
            .expect("notification state lock")
            .clone()
            .expect("second notification");

        assert_eq!(second.message, "second");
        assert!(second.expires_at >= first.expires_at);
        assert!(second.expires_at > Instant::now() + Duration::from_secs(2));
    }

    #[test]
    fn notification_anchor_tracks_current_suggestion_geometry() {
        let base = SuggestionViewportSpec {
            state: PopupState::default(),
            prefer_root_anchor: false,
            x: 120,
            y: 240,
            height: 72,
        };
        let base_anchor = suggestion_notification_anchor(&base);
        assert_eq!(base_anchor.x(), 120.0);
        assert_eq!(base_anchor.y(), 240.0);
        assert_eq!(base_anchor.height(), 72.0);

        let resized = SuggestionViewportSpec {
            x: 180,
            y: 210,
            height: 144,
            ..base
        };
        let resized_anchor = suggestion_notification_anchor(&resized);
        assert_eq!(resized_anchor.x(), 180.0);
        assert_eq!(resized_anchor.y(), 210.0);
        assert_eq!(resized_anchor.height(), 144.0);
    }

    #[test]
    fn embedded_window_icon_decodes_without_runtime_assets() {
        let icon = load_window_icon().expect("embedded window icon must decode");
        assert!(icon.width > 0);
        assert!(icon.height > 0);
        assert!(!icon.rgba.is_empty());
    }

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
