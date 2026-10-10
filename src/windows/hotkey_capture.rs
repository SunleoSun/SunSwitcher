use std::sync::{Arc, Mutex};

use eframe::egui;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyNameTextW, MAPVK_VK_TO_VSC, MapVirtualKeyW, VK_CONTROL, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_MENU, VK_RCONTROL, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT,
};

use crate::persistence::{HotkeyAction, HotkeyBinding};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotkeyCaptureState {
    Idle,
    Listening(HotkeyAction),
    Pending {
        action: HotkeyAction,
        binding: HotkeyBinding,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HotkeyCaptureEffect {
    Inactive,
    Consumed,
    Captured,
}

#[derive(Clone)]
pub struct HotkeyCaptureHandle {
    state: Arc<Mutex<HotkeyCaptureState>>,
    repaint_ctx: egui::Context,
}

impl HotkeyCaptureHandle {
    pub(crate) fn new(state: Arc<Mutex<HotkeyCaptureState>>, repaint_ctx: egui::Context) -> Self {
        Self { state, repaint_ctx }
    }

    pub(crate) fn shared_state() -> Arc<Mutex<HotkeyCaptureState>> {
        Arc::new(Mutex::new(HotkeyCaptureState::Idle))
    }

    pub fn begin(&self, action: HotkeyAction) {
        if let Ok(mut state) = self.state.lock() {
            *state = HotkeyCaptureState::Listening(action);
            crate::runtime_log!("[hotkey-capture] listening action={action:?}");
        }
        self.repaint_ctx.request_repaint();
    }

    pub fn cancel(&self) {
        if let Ok(mut state) = self.state.lock() {
            if !matches!(*state, HotkeyCaptureState::Idle) {
                crate::runtime_log!("[hotkey-capture] canceled state={state:?}");
            }
            *state = HotkeyCaptureState::Idle;
        }
        self.repaint_ctx.request_repaint();
    }

    pub(crate) fn state(&self) -> HotkeyCaptureState {
        self.state
            .lock()
            .map(|state| *state)
            .unwrap_or(HotkeyCaptureState::Idle)
    }

    pub(crate) fn is_active(&self) -> bool {
        !matches!(self.state(), HotkeyCaptureState::Idle)
    }

    pub(crate) fn observe_key_down(
        &self,
        vk_code: u32,
        control: bool,
        shift: bool,
        alt: bool,
        win: bool,
    ) -> HotkeyCaptureEffect {
        let Ok(mut state) = self.state.lock() else {
            return HotkeyCaptureEffect::Inactive;
        };
        let HotkeyCaptureState::Listening(action) = *state else {
            return HotkeyCaptureEffect::Inactive;
        };
        if is_modifier_vk(vk_code) {
            return HotkeyCaptureEffect::Consumed;
        }
        if vk_code == 0x1B {
            *state = HotkeyCaptureState::Idle;
            drop(state);
            crate::runtime_log!("[hotkey-capture] escape canceled capture");
            self.repaint_ctx.request_repaint();
            return HotkeyCaptureEffect::Consumed;
        }
        let Ok(key_code) = u16::try_from(vk_code) else {
            return HotkeyCaptureEffect::Consumed;
        };
        let Ok(binding) = HotkeyBinding::try_new(key_code, control, shift, alt, win) else {
            return HotkeyCaptureEffect::Consumed;
        };
        *state = HotkeyCaptureState::Pending { action, binding };
        drop(state);
        crate::runtime_log!(
            "[hotkey-capture] captured action={action:?} binding={}",
            format_hotkey(binding)
        );
        self.repaint_ctx.request_repaint();
        HotkeyCaptureEffect::Captured
    }
}

fn is_modifier_vk(vk_code: u32) -> bool {
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
    .into_iter()
    .any(|key| vk_code == key as u32)
}

pub(crate) fn format_hotkey(binding: HotkeyBinding) -> String {
    let mut parts = Vec::with_capacity(5);
    let modifiers = binding.modifiers();
    if modifiers.control() {
        parts.push("Ctrl".to_owned());
    }
    if modifiers.shift() {
        parts.push("Shift".to_owned());
    }
    if modifiers.alt() {
        parts.push("Alt".to_owned());
    }
    if modifiers.win() {
        parts.push("Win".to_owned());
    }
    parts.push(format_key(binding.key_code()));
    parts.join("+")
}

fn format_key(key_code: u16) -> String {
    match key_code {
        0x08 => return "Backspace".to_owned(),
        0x09 => return "Tab".to_owned(),
        0x0D => return "Enter".to_owned(),
        0x13 => return "Pause".to_owned(),
        0x20 => return "Space".to_owned(),
        0x21 => return "Page Up".to_owned(),
        0x22 => return "Page Down".to_owned(),
        0x23 => return "End".to_owned(),
        0x24 => return "Home".to_owned(),
        0x25 => return "Left".to_owned(),
        0x26 => return "Up".to_owned(),
        0x27 => return "Right".to_owned(),
        0x28 => return "Down".to_owned(),
        0x2D => return "Insert".to_owned(),
        0x2E => return "Delete".to_owned(),
        0x6A => return "Num*".to_owned(),
        0x6B => return "Num+".to_owned(),
        0x6D => return "Num-".to_owned(),
        0x6F => return "Num/".to_owned(),
        0xBB => return "+".to_owned(),
        0xBD => return "-".to_owned(),
        _ => {}
    }
    if (b'0' as u16..=b'9' as u16).contains(&key_code)
        || (b'A' as u16..=b'Z' as u16).contains(&key_code)
    {
        return char::from_u32(u32::from(key_code))
            .map(|character| character.to_string())
            .unwrap_or_else(|| format!("VK {key_code}"));
    }
    if (0x70..=0x87).contains(&key_code) {
        return format!("F{}", key_code - 0x6F);
    }

    let scan_code = unsafe { MapVirtualKeyW(u32::from(key_code), MAPVK_VK_TO_VSC) };
    if scan_code != 0 {
        let l_param = i32::try_from(scan_code << 16).unwrap_or_default();
        let mut buffer = [0u16; 64];
        let length = unsafe { GetKeyNameTextW(l_param, buffer.as_mut_ptr(), buffer.len() as i32) };
        if length > 0 {
            return String::from_utf16_lossy(&buffer[..length as usize]);
        }
    }
    format!("VK {key_code}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_hotkey_uses_readable_default_names() {
        let binding = HotkeyBinding::try_new(0x27, false, false, true, false).unwrap();
        assert_eq!(format_hotkey(binding), "Alt+Right");
    }

    #[test]
    fn ctrl_break_capture_is_canonical_ctrl_pause() {
        let handle = HotkeyCaptureHandle::new(
            HotkeyCaptureHandle::shared_state(),
            egui::Context::default(),
        );
        handle.begin(HotkeyAction::IgnoreWord);

        assert_eq!(
            handle.observe_key_down(0x03, true, false, false, false),
            HotkeyCaptureEffect::Captured
        );
        let HotkeyCaptureState::Pending { action, binding } = handle.state() else {
            panic!("Ctrl+Pause capture must produce a pending binding");
        };
        assert_eq!(action, HotkeyAction::IgnoreWord);
        assert_eq!(binding, HotkeyBinding::IGNORE_WORD_DEFAULT);
        assert_eq!(format_hotkey(binding), "Ctrl+Pause");
    }

    #[test]
    fn capture_waits_for_non_modifier_and_exposes_pending_binding_for_confirmation() {
        let handle = HotkeyCaptureHandle::new(
            HotkeyCaptureHandle::shared_state(),
            egui::Context::default(),
        );
        handle.begin(HotkeyAction::ClipboardCurrent);
        assert_eq!(
            handle.observe_key_down(VK_CONTROL as u32, true, false, false, false),
            HotkeyCaptureEffect::Consumed
        );
        assert_eq!(
            handle.state(),
            HotkeyCaptureState::Listening(HotkeyAction::ClipboardCurrent)
        );

        assert_eq!(
            handle.observe_key_down(b'J' as u32, true, true, false, false),
            HotkeyCaptureEffect::Captured
        );
        assert_eq!(
            handle.state(),
            HotkeyCaptureState::Pending {
                action: HotkeyAction::ClipboardCurrent,
                binding: HotkeyBinding::try_new(b'J' as u16, true, true, false, false).unwrap(),
            }
        );
    }

    #[test]
    fn escape_cancels_an_active_capture() {
        let handle = HotkeyCaptureHandle::new(
            HotkeyCaptureHandle::shared_state(),
            egui::Context::default(),
        );
        handle.begin(HotkeyAction::IgnoreWord);
        assert_eq!(
            handle.observe_key_down(0x1B, false, false, false, false),
            HotkeyCaptureEffect::Consumed
        );
        assert_eq!(handle.state(), HotkeyCaptureState::Idle);
    }
}
