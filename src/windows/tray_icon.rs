use std::mem::{size_of, zeroed};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_SETVERSION, NOTIFYICON_VERSION_4,
    NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, IDI_APPLICATION, LoadIconW, MSG, PostThreadMessageW,
    RegisterClassW, TranslateMessage, WM_APP, WM_DESTROY, WM_LBUTTONUP, WM_QUIT, WM_RBUTTONUP,
    WNDCLASSW, WS_OVERLAPPED,
};

use super::clipboard_manager::{ClipboardManagerHandle, ClipboardManagerTab};

const TRAY_ID: u32 = 1;
const WM_TRAY_ICON: u32 = WM_APP + 31;
const TRAY_READY_TIMEOUT: Duration = Duration::from_secs(2);

static TRAY_CLIPBOARD: Mutex<Option<ClipboardManagerHandle>> = Mutex::new(None);

pub(crate) struct TrayIcon {
    thread_id: std::sync::Arc<AtomicU32>,
    worker: Option<JoinHandle<()>>,
}

impl TrayIcon {
    pub(crate) fn start(clipboard: ClipboardManagerHandle) -> Self {
        if let Ok(mut slot) = TRAY_CLIPBOARD.lock() {
            *slot = Some(clipboard);
        }

        let thread_id = std::sync::Arc::new(AtomicU32::new(0));
        let worker_thread_id = std::sync::Arc::clone(&thread_id);
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
        if let Ok(mut slot) = TRAY_CLIPBOARD.lock() {
            *slot = None;
        }
    }
}

unsafe fn run_tray_loop(thread_id: std::sync::Arc<AtomicU32>, ready_sender: mpsc::SyncSender<()>) {
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

    let mut nid = tray_data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
    nid.Anonymous.uVersion = NOTIFYICON_VERSION_4;
    unsafe { Shell_NotifyIconW(NIM_SETVERSION, &nid) };
    let _ = ready_sender.send(());

    let mut message: MSG = unsafe { zeroed() };
    while unsafe { GetMessageW(&mut message, null_mut(), 0, 0) } > 0 {
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }

    let delete_data = tray_data(hwnd);
    unsafe { Shell_NotifyIconW(NIM_DELETE, &delete_data) };
    unsafe { DestroyWindow(hwnd) };
}

unsafe extern "system" fn tray_window_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if message == WM_TRAY_ICON
        && wparam as u32 == TRAY_ID
        && matches!(lparam as u32, WM_LBUTTONUP | WM_RBUTTONUP)
    {
        if let Ok(slot) = TRAY_CLIPBOARD.lock()
            && let Some(clipboard) = slot.as_ref()
        {
            clipboard.show(ClipboardManagerTab::Current, 0);
        }
        return 0;
    }
    if message == WM_DESTROY {
        return 0;
    }
    unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
}

fn tray_data(hwnd: HWND) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
    data.uCallbackMessage = WM_TRAY_ICON;
    data.hIcon = unsafe { LoadIconW(null_mut(), IDI_APPLICATION) };
    write_wide_fixed(&mut data.szTip, "SunSwitcher");
    data
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
