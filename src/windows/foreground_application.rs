//! Process inspection belongs only to this Windows boundary. Results (including
//! failures) are cached until the foreground HWND changes, never retried per key.
use std::sync::Mutex;

use windows_sys::Win32::Foundation::CloseHandle;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowThreadProcessId;

use crate::application_policy::ApplicationIdentity;

#[derive(Default)]
struct ForegroundApplicationCache {
    window: Option<usize>,
    application: Option<ApplicationIdentity>,
    recent_external: Option<ApplicationIdentity>,
}

impl ForegroundApplicationCache {
    fn observe(
        &mut self,
        window: usize,
        resolve: impl FnOnce(usize) -> Option<(ApplicationIdentity, bool)>,
    ) -> Option<ApplicationIdentity> {
        if self.window != Some(window) {
            self.window = Some(window);
            self.application = resolve(window).map(|(application, external)| {
                if external {
                    self.recent_external = Some(application.clone());
                }
                application
            });
        }
        self.application.clone()
    }
}

static FOREGROUND: Mutex<ForegroundApplicationCache> = Mutex::new(ForegroundApplicationCache {
    window: None,
    application: None,
    recent_external: None,
});

pub(super) fn observe_foreground_window(window: usize) -> Option<ApplicationIdentity> {
    FOREGROUND.lock().ok()?.observe(window, resolve_application)
}

pub(super) fn recent_external_application() -> Option<ApplicationIdentity> {
    FOREGROUND.lock().ok()?.recent_external.clone()
}

fn resolve_application(window: usize) -> Option<(ApplicationIdentity, bool)> {
    if window == 0 {
        return None;
    }
    let mut process_id = 0;
    unsafe { GetWindowThreadProcessId(window as _, &mut process_id) };
    if process_id == 0 {
        return None;
    }
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, process_id) };
    if process.is_null() {
        return None;
    }
    // Windows' maximum extended-length path, including the terminator.
    let mut path = vec![0u16; 32768];
    let mut length = path.len() as u32;
    let resolved = unsafe {
        QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, path.as_mut_ptr(), &mut length)
    };
    unsafe { CloseHandle(process) };
    if resolved == 0 {
        return None;
    }
    let mut current_process_id = 0;
    unsafe { GetWindowThreadProcessId(window as _, &mut current_process_id) };
    if current_process_id != process_id {
        return None;
    }
    let path = String::from_utf16(path.get(..length as usize)?).ok()?;
    let application = ApplicationIdentity::try_new(&path).ok()?;
    Some((application, process_id != unsafe { GetCurrentProcessId() }))
}

#[cfg(test)]
#[path = "foreground_application_certification.rs"]
mod certification;
