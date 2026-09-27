use std::env;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use eframe::egui;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreatePopupMenu, CreateWindowExW,
    DefWindowProcW, DestroyIcon, DestroyMenu, DestroyWindow, DispatchMessageW, GetCursorPos,
    GetForegroundWindow, GetMessageW, IDI_APPLICATION, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE,
    LoadIconW, LoadImageW, MF_STRING, MSG, PostThreadMessageW, RegisterClassW, SetForegroundWindow,
    TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_CONTEXTMENU,
    WM_DESTROY, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONUP, WNDCLASSW, WS_OVERLAPPED,
};

use super::clipboard_manager::{ClipboardManagerHandle, ClipboardManagerTab};
use super::keyboard_runtime::request_global_keyboard_hook_stop;

const TRAY_ID: u32 = 1;
const WM_TRAY_ICON: u32 = WM_APP + 31;
const NIN_SELECT: u32 = 0x0400;
const NIN_KEYSELECT: u32 = 0x0401;
const TRAY_READY_TIMEOUT: Duration = Duration::from_secs(2);
const CLIPBOARD_ICON_PATH: &str = "assets/clipboard.ico";
const TRAY_EXIT_COMMAND: usize = 1001;

#[derive(Clone)]
struct TrayRuntimeState {
    clipboard: ClipboardManagerHandle,
    shutdown: Arc<AtomicBool>,
    repaint_ctx: egui::Context,
}

static TRAY_STATE: Mutex<Option<TrayRuntimeState>> = Mutex::new(None);

pub(crate) struct TrayIcon {
    thread_id: Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
}

impl TrayIcon {
    pub(crate) fn start(
        clipboard: ClipboardManagerHandle,
        shutdown: Arc<AtomicBool>,
        repaint_ctx: egui::Context,
    ) -> Self {
        if let Ok(mut slot) = TRAY_STATE.lock() {
            *slot = Some(TrayRuntimeState {
                clipboard,
                shutdown,
                repaint_ctx,
            });
        }

        let thread_id = Arc::new(AtomicU32::new(0));
        let worker_thread_id = Arc::clone(&thread_id);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || unsafe {
            run_tray_loop(worker_thread_id, ready_sender);
        });
        let _ = ready_receiver.recv_timeout(TRAY_READY_TIMEOUT);

        Self {
            thread_id,
            worker: Some(worker),
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        let thread_id = self.thread_id.load(Ordering::Acquire);
        if thread_id != 0 {
            unsafe {
                PostThreadMessageW(thread_id, WM_QUIT, 0, 0);
            }
        }
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        if let Ok(mut slot) = TRAY_STATE.lock() {
            *slot = None;
        }
    }
}

unsafe fn run_tray_loop(thread_id: Arc<AtomicU32>, ready_sender: mpsc::SyncSender<()>) {
    thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::Release);

    let class_name = wide_null("SunSwitcherTrayWindow");
    let window_title = wide_null("SunSwitcher tray");
    let instance = unsafe { GetModuleHandleW(null()) };
    let class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(tray_window_proc),
        hInstance: instance,
        lpszClassName: class_name.as_ptr(),
        ..unsafe { zeroed() }
    };
    unsafe { RegisterClassW(&class) };

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            window_title.as_ptr(),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            null_mut(),
            null_mut(),
            instance,
            null(),
        )
    };

    if hwnd.is_null() {
        let _ = ready_sender.send(());
        return;
    }

    let icon = TrayIconResource::load();
    let nid = tray_data(hwnd, icon.handle());
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    let _ = ready_sender.send(());

    let mut message: MSG = unsafe { zeroed() };
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    let delete_data = tray_data(hwnd, icon.handle());
    unsafe { Shell_NotifyIconW(NIM_DELETE, &delete_data) };
    unsafe { DestroyWindow(hwnd) };
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY_ICON {
        match (lparam as u32) & 0xFFFF {
            WM_LBUTTONUP | NIN_SELECT | NIN_KEYSELECT => {
                let target_window_id = unsafe { GetForegroundWindow() } as usize;
                unsafe { SetForegroundWindow(hwnd) };
                open_clipboard_from_tray(target_window_id);
                return 0;
            }
            WM_RBUTTONUP | WM_CONTEXTMENU => {
                unsafe { show_tray_menu(hwnd) };
                return 0;
            }
            _ => {}
        }
    }
    if message == WM_DESTROY {
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn open_clipboard_from_tray(target_window_id: usize) {
    if let Ok(slot) = TRAY_STATE.lock()
        && let Some(state) = slot.as_ref()
    {
        state
            .clipboard
            .show(ClipboardManagerTab::Current, target_window_id);
    }
}

unsafe fn show_tray_menu(hwnd: HWND) {
    let menu = unsafe { CreatePopupMenu() };
    if menu.is_null() {
        return;
    }
    let exit_label = wide_null("Exit");
    unsafe { AppendMenuW(menu, MF_STRING, TRAY_EXIT_COMMAND, exit_label.as_ptr()) };
    let mut point: POINT = unsafe { zeroed() };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        unsafe { DestroyMenu(menu) };
        return;
    }
    unsafe { SetForegroundWindow(hwnd) };
    let command = unsafe {
        TrackPopupMenu(
            menu,
            TPM_RIGHTBUTTON | TPM_RETURNCMD,
            point.x,
            point.y,
            0,
            hwnd,
            null(),
        )
    } as usize;
    unsafe { DestroyMenu(menu) };
    if command == TRAY_EXIT_COMMAND {
        request_exit_from_tray();
    }
}

fn request_exit_from_tray() {
    let _ = request_global_keyboard_hook_stop();
    if let Ok(slot) = TRAY_STATE.lock()
        && let Some(state) = slot.as_ref()
    {
        state.shutdown.store(true, Ordering::Release);
        state.repaint_ctx.request_repaint();
    }
}

fn tray_data(hwnd: HWND, icon: *mut c_void) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY_ICON;
    data.hIcon = icon;
    write_wide_fixed(&mut data.szTip, "SunSwitcher");
    data
}

struct TrayIconResource {
    handle: *mut c_void,
    owned: bool,
}

impl TrayIconResource {
    fn load() -> Self {
        for path in clipboard_icon_candidates() {
            if !path.exists() {
                continue;
            }
            let wide_path = wide_null(&path.to_string_lossy());
            let handle = unsafe {
                LoadImageW(
                    null_mut(),
                    wide_path.as_ptr(),
                    IMAGE_ICON,
                    0,
                    0,
                    LR_LOADFROMFILE | LR_DEFAULTSIZE,
                )
            };
            if !handle.is_null() {
                return Self {
                    handle,
                    owned: true,
                };
            }
        }
        Self {
            handle: unsafe { LoadIconW(null_mut(), IDI_APPLICATION) },
            owned: false,
        }
    }

    fn handle(&self) -> *mut c_void {
        self.handle
    }
}

impl Drop for TrayIconResource {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_null() {
            unsafe {
                DestroyIcon(self.handle);
            }
        }
    }
}

fn clipboard_icon_candidates() -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Ok(current_dir) = env::current_dir() {
        paths.push(current_dir.join(CLIPBOARD_ICON_PATH));
    }
    if let Ok(executable) = env::current_exe()
        && let Some(directory) = executable.parent()
    {
        paths.push(directory.join(CLIPBOARD_ICON_PATH));
    }
    paths
}

fn wide_null(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn write_wide_fixed(target: &mut [u16], text: &str) {
    for (slot, value) in target
        .iter_mut()
        .zip(text.encode_utf16().chain(std::iter::once(0)))
    {
        *slot = value;
    }
}
