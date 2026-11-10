use std::ptr::{null, null_mut};

use sun_observations::certifications::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow};

use super::*;

#[test]
fn foreground_resolution_is_cached_including_failure_and_preserves_recent_external_application() {
    let mut c = certification("foreground identity cache lifecycle");
    let application = ApplicationIdentity::try_new(r"C:\Apps\example.exe").unwrap();
    let mut cache = ForegroundApplicationCache::default();
    c.check_fact_expect(
        "initial external identity",
        cache.observe(1, |_| Some((application.clone(), true))),
        Some(application.clone()),
    );
    c.check_fact_expect(
        "ordinary key does not query process",
        cache.observe(1, |_| panic!("process lookup on unchanged foreground")),
        Some(application.clone()),
    );
    c.check_fact_expect(
        "unresolved foreground clears active identity",
        cache.observe(2, |_| None),
        None,
    );
    c.check_fact_expect(
        "failure is cached",
        cache.observe(2, |_| panic!("process retry on unchanged foreground")),
        None,
    );
    c.check_fact_expect(
        "recent external survives lookup failure",
        cache.recent_external.clone(),
        Some(application.clone()),
    );
    let own = ApplicationIdentity::try_new(r"C:\Apps\SunSwitcher.exe").unwrap();
    cache.observe(3, |_| Some((own, false)));
    c.check_fact_expect(
        "SunSwitcher does not replace recent external",
        cache.recent_external.clone(),
        Some(application.clone()),
    );
    c.check_fact_expect(
        "returning to application resolves again",
        cache.observe(1, |_| Some((application.clone(), true))),
        Some(application),
    );
    c.assert_certify();
}

#[test]
fn windows_process_boundary_resolves_the_full_executable_path_of_an_owned_window() {
    let mut c = certification("Windows foreground executable identity");
    let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
    let window = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            null(),
            0,
            0,
            0,
            1,
            1,
            null_mut(),
            null_mut(),
            null_mut(),
            null(),
        )
    };
    let resolved = resolve_application(window as usize);
    if !window.is_null() {
        unsafe { DestroyWindow(window) };
    }
    c.check_fact_expect("native test window created", !window.is_null(), true);
    let expected =
        ApplicationIdentity::try_new(std::env::current_exe().unwrap().to_str().unwrap()).unwrap();
    c.check_fact_expect(
        "actual process path and own-process identity",
        resolved,
        Some((expected, false)),
    );
    c.check_fact_expect("missing HWND is unresolved", resolve_application(0), None);
    c.assert_certify();
}
