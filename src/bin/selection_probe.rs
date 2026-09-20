#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("selection_probe is available only on Windows");
}

#[cfg(target_os = "windows")]
mod windows_probe {
    use std::mem::zeroed;
    use std::ptr::null_mut;

    use sunswitcher::replacement::{
        SelectedReplacementEngine, SelectedReplacementText, SelectedTextDecision,
    };
    use sunswitcher::windows::{SelectedTextRuntimeError, SelectedTextSession};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_F8,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    const HOTKEY_ID: i32 = 0x5353;

    pub fn run() {
        println!("SunSwitcher selected-text replacement probe");
        println!(
            "1. Put something valuable in the clipboard: text, a copied file, or a screenshot/image."
        );
        println!("2. Select text in another application and press F8.");
        println!("3. The selection should be wrapped with [SunSwitcher] markers.");
        println!("4. Paste normally afterward: the original clipboard content should still work.");
        println!("Press Ctrl+C in this console to stop the probe.");

        let registered =
            unsafe { RegisterHotKey(null_mut(), HOTKEY_ID, MOD_NOREPEAT, VK_F8 as u32) };
        if registered == 0 {
            eprintln!("Could not register F8 as a global probe hotkey.");
            std::process::exit(1);
        }

        unsafe {
            let mut message: MSG = zeroed();
            loop {
                let result = GetMessageW(&mut message, null_mut(), 0, 0);
                if result <= 0 {
                    break;
                }
                if message.message == WM_HOTKEY && message.wParam as i32 == HOTKEY_ID {
                    run_once();
                }
            }
            UnregisterHotKey(null_mut(), HOTKEY_ID);
        }
    }

    fn run_once() {
        let session = match SelectedTextSession::capture() {
            Ok(Some(session)) => session,
            Ok(None) => {
                println!(
                    "[SELECTION] no text selection detected; supported clipboard state was restored"
                );
                return;
            }
            Err(SelectedTextRuntimeError::UnsupportedClipboardState) => {
                eprintln!(
                    "[SELECTION] capture skipped: clipboard type is not supported yet; text, copied files, and DIB/DIBV5 images are supported"
                );
                return;
            }
            Err(error) => {
                eprintln!("[SELECTION] capture failed: {error:?}");
                return;
            }
        };

        let source = session.selected_text().clone();
        let replacement = format!("[SunSwitcher]{}[/SunSwitcher]", source.as_str());
        let decision = SelectedTextDecision::Replace(
            SelectedReplacementText::try_new(replacement).expect("probe replacement is valid text"),
        );
        let Some(action) = SelectedReplacementEngine::new().plan(source, decision) else {
            if let Err(error) = session.finish_without_replacement() {
                eprintln!("[SELECTION] finish failed: {error:?}");
            }
            return;
        };

        let source_chars = action.source().as_str().chars().count();
        let replacement_chars = action.replacement().as_str().chars().count();
        match session.apply(&action) {
            Ok(()) => println!(
                "[SELECTION] replaced {source_chars} chars with {replacement_chars} chars by direct Unicode injection; clipboard was restored before replacement"
            ),
            Err(error) => eprintln!("[SELECTION] replacement failed: {error:?}"),
        }
    }
}

#[cfg(target_os = "windows")]
fn main() {
    windows_probe::run();
}
