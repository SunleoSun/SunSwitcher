use std::collections::VecDeque;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::slice;

use uiautomation::UIAutomation;
use uiautomation::patterns::{UITextEditPattern, UITextPattern, UITextRange};
use uiautomation::types::{TextPatternRangeEndpoint, TextUnit};
use windows::Win32::System::Ole::{
    SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
    SafeArrayUnaccessData,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Accessibility::{AccessibleObjectFromWindow, IAccessible};
use windows::core::Interface as _;
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{
    ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GUITHREADINFO, GetCursorPos, GetForegroundWindow, GetGUIThreadInfo, GetWindowRect, OBJID_CARET,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretSource {
    UiAutomationCaret,
    UiAutomationTextEdit,
    UiAutomationSelection,
    MsaaCaret,
    Win32GuiThread,
    PointerFallback,
    ForegroundWindowFallback,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CaretAnchor {
    x: f32,
    y: f32,
    height: f32,
    source: CaretSource,
}

impl CaretAnchor {
    pub const fn new(x: f32, y: f32, height: f32, source: CaretSource) -> Self {
        Self {
            x,
            y,
            height,
            source,
        }
    }

    pub const fn x(self) -> f32 {
        self.x
    }

    pub const fn y(self) -> f32 {
        self.y
    }

    pub const fn height(self) -> f32 {
        self.height
    }

    pub const fn source(self) -> CaretSource {
        self.source
    }
}

#[derive(Debug)]
pub struct CaretLocator {
    automation: Option<UIAutomation>,
}

impl Default for CaretLocator {
    fn default() -> Self {
        Self::new()
    }
}

impl CaretLocator {
    pub fn new() -> Self {
        Self {
            automation: UIAutomation::new().ok(),
        }
    }

    pub fn locate(&self) -> Option<CaretAnchor> {
        self.locate_via_uia_caret()
            .or_else(|| self.locate_via_uia_text_edit())
            .or_else(|| self.locate_via_uia_selection())
            .or_else(locate_via_msaa_caret)
            .or_else(locate_via_gui_thread)
    }

    pub fn fallback_anchor(&self) -> Option<CaretAnchor> {
        locate_via_pointer().or_else(locate_via_foreground_window)
    }

    pub fn selected_text(&self) -> Option<String> {
        let pattern = self.focused_text_pattern()?;
        pattern
            .get_selection()
            .ok()?
            .into_iter()
            .filter_map(|range| range.get_text(-1).ok())
            .find(|text| !text.is_empty())
    }

    fn focused_text_pattern(&self) -> Option<UITextPattern> {
        let automation = self.automation.as_ref()?;
        let focused = automation.get_focused_element().ok()?;
        if let Ok(pattern) = focused.get_pattern::<UITextPattern>() {
            return Some(pattern);
        }

        let walker = automation.get_raw_view_walker().ok()?;
        let mut element = focused.clone();
        for _ in 0..8 {
            let Ok(parent) = walker.get_parent(&element) else {
                break;
            };
            element = parent;
            if let Ok(pattern) = element.get_pattern::<UITextPattern>() {
                return Some(pattern);
            }
        }

        let mut queue = VecDeque::new();
        if let Some(children) = walker.get_children(&focused) {
            for child in children {
                queue.push_back((child, 1usize));
            }
        }
        let mut visited = 0usize;
        while let Some((element, depth)) = queue.pop_front() {
            if visited >= 128 {
                break;
            }
            visited += 1;
            if let Ok(pattern) = element.get_pattern::<UITextPattern>() {
                return Some(pattern);
            }
            if visited >= 128 || depth >= 8 {
                continue;
            }
            if let Some(children) = walker.get_children(&element) {
                for child in children {
                    queue.push_back((child, depth + 1));
                }
            }
        }
        None
    }

    fn focused_text_edit_pattern(&self) -> Option<UITextEditPattern> {
        let automation = self.automation.as_ref()?;
        let focused = automation.get_focused_element().ok()?;
        if let Ok(pattern) = focused.get_pattern::<UITextEditPattern>() {
            return Some(pattern);
        }

        let walker = automation.get_raw_view_walker().ok()?;
        let mut element = focused.clone();
        for _ in 0..8 {
            let Ok(parent) = walker.get_parent(&element) else {
                break;
            };
            element = parent;
            if let Ok(pattern) = element.get_pattern::<UITextEditPattern>() {
                return Some(pattern);
            }
        }

        let mut queue = VecDeque::new();
        if let Some(children) = walker.get_children(&focused) {
            for child in children {
                queue.push_back((child, 1usize));
            }
        }
        let mut visited = 0usize;
        while let Some((element, depth)) = queue.pop_front() {
            if visited >= 128 {
                break;
            }
            visited += 1;
            if let Ok(pattern) = element.get_pattern::<UITextEditPattern>() {
                return Some(pattern);
            }
            if visited >= 128 || depth >= 8 {
                continue;
            }
            if let Some(children) = walker.get_children(&element) {
                for child in children {
                    queue.push_back((child, depth + 1));
                }
            }
        }
        None
    }

    fn locate_via_uia_caret(&self) -> Option<CaretAnchor> {
        let pattern = self.focused_text_pattern()?;
        let (active, range) = pattern.get_caret_range().ok()?;
        if !active {
            return None;
        }
        caret_range_anchor(&range, CaretSource::UiAutomationCaret)
    }

    fn locate_via_uia_text_edit(&self) -> Option<CaretAnchor> {
        let pattern = self.focused_text_edit_pattern()?;
        let text_pattern: &UITextPattern = pattern.as_ref();
        let (active, range) = text_pattern.get_caret_range().ok()?;
        if !active {
            return None;
        }
        caret_range_anchor(&range, CaretSource::UiAutomationTextEdit)
    }

    fn locate_via_uia_selection(&self) -> Option<CaretAnchor> {
        let pattern = self.focused_text_pattern()?;
        let range = pattern.get_selection().ok()?.into_iter().next()?;
        if let Some(anchor) = caret_range_anchor(&range, CaretSource::UiAutomationSelection) {
            return Some(anchor);
        }
        let expanded = range.clone();
        let _ = expanded.expand_to_enclosing_unit(TextUnit::Character);
        range_anchor(&expanded, CaretSource::UiAutomationSelection)
    }
}

fn caret_range_anchor(range: &UITextRange, source: CaretSource) -> Option<CaretAnchor> {
    if let Some(anchor) = range_anchor(range, source) {
        return Some(anchor);
    }

    let previous = range.clone();
    if previous
        .move_endpoint_by_unit(TextPatternRangeEndpoint::Start, TextUnit::Character, -1)
        .ok()
        .is_some_and(|moved| moved != 0)
        && let Some((x, y, width, height)) = range_rect(&previous)
    {
        return Some(CaretAnchor::new(
            (x + width) as f32,
            y as f32,
            height.max(1.0) as f32,
            source,
        ));
    }

    let next = range.clone();
    if next
        .move_endpoint_by_unit(TextPatternRangeEndpoint::End, TextUnit::Character, 1)
        .ok()
        .is_some_and(|moved| moved != 0)
        && let Some((x, y, _width, height)) = range_rect(&next)
    {
        return Some(CaretAnchor::new(
            x as f32,
            y as f32,
            height.max(1.0) as f32,
            source,
        ));
    }

    None
}

fn range_rect(range: &UITextRange) -> Option<(f64, f64, f64, f64)> {
    let safe_array = unsafe { range.as_ref().GetBoundingRectangles().ok()? };
    if safe_array.is_null() {
        return None;
    }

    let result = unsafe {
        let lower = match SafeArrayGetLBound(safe_array, 1) {
            Ok(value) => value,
            Err(_) => {
                let _ = SafeArrayDestroy(safe_array);
                return None;
            }
        };
        let upper = match SafeArrayGetUBound(safe_array, 1) {
            Ok(value) => value,
            Err(_) => {
                let _ = SafeArrayDestroy(safe_array);
                return None;
            }
        };
        let length = upper.saturating_sub(lower).saturating_add(1) as usize;
        if length < 4 {
            let _ = SafeArrayDestroy(safe_array);
            return None;
        }

        let mut data: *mut c_void = std::ptr::null_mut();
        if SafeArrayAccessData(safe_array, &mut data).is_err() || data.is_null() {
            let _ = SafeArrayDestroy(safe_array);
            return None;
        }
        let values = slice::from_raw_parts(data.cast::<f64>(), length);
        let rect = (values[0], values[1], values[2], values[3]);
        let _ = SafeArrayUnaccessData(safe_array);
        let _ = SafeArrayDestroy(safe_array);
        rect
    };

    let (x, y, width, height) = result;
    if !x.is_finite()
        || !y.is_finite()
        || !width.is_finite()
        || !height.is_finite()
        || width < 0.0
        || height <= 0.0
    {
        return None;
    }
    Some(result)
}

fn range_anchor(range: &UITextRange, source: CaretSource) -> Option<CaretAnchor> {
    let (x, y, _width, height) = range_rect(range)?;
    Some(CaretAnchor::new(
        x as f32,
        y as f32,
        height.max(1.0) as f32,
        source,
    ))
}

fn locate_via_msaa_caret() -> Option<CaretAnchor> {
    let mut info: GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    let focused_window = if unsafe { GetGUIThreadInfo(0, &mut info) } != 0 {
        info.hwndFocus
    } else {
        std::ptr::null_mut()
    };
    let foreground_window = unsafe { GetForegroundWindow() };

    for raw_window in [focused_window, foreground_window] {
        if raw_window.is_null() {
            continue;
        }

        let window = windows::Win32::Foundation::HWND(raw_window);
        let mut raw_accessible = std::ptr::null_mut();
        if unsafe {
            AccessibleObjectFromWindow(
                window,
                OBJID_CARET as u32,
                &IAccessible::IID,
                &mut raw_accessible,
            )
        }
        .is_err()
            || raw_accessible.is_null()
        {
            continue;
        }
        let accessible = unsafe { IAccessible::from_raw(raw_accessible) };

        let mut child = VARIANT::default();
        unsafe {
            (*child.Anonymous.Anonymous).vt = VT_I4;
            (*child.Anonymous.Anonymous).Anonymous.lVal = 0;
        }

        let mut x = 0;
        let mut y = 0;
        let mut width = 0;
        let mut height = 0;
        if unsafe { accessible.accLocation(&mut x, &mut y, &mut width, &mut height, &child) }
            .is_err()
            || width < 0
            || height <= 0
        {
            continue;
        }

        return Some(CaretAnchor::new(
            x as f32,
            y as f32,
            height.max(1) as f32,
            CaretSource::MsaaCaret,
        ));
    }

    None
}

fn locate_via_gui_thread() -> Option<CaretAnchor> {
    let mut info: GUITHREADINFO = unsafe { zeroed() };
    info.cbSize = size_of::<GUITHREADINFO>() as u32;
    if unsafe { GetGUIThreadInfo(0, &mut info) } == 0 || info.hwndCaret.is_null() {
        return None;
    }

    let mut top_left = POINT {
        x: info.rcCaret.left,
        y: info.rcCaret.top,
    };
    let mut bottom_right = POINT {
        x: info.rcCaret.right,
        y: info.rcCaret.bottom,
    };
    if unsafe { ClientToScreen(info.hwndCaret, &mut top_left) } == 0
        || unsafe { ClientToScreen(info.hwndCaret, &mut bottom_right) } == 0
    {
        return None;
    }

    Some(CaretAnchor::new(
        top_left.x as f32,
        top_left.y as f32,
        (bottom_right.y - top_left.y).max(1) as f32,
        CaretSource::Win32GuiThread,
    ))
}

fn locate_via_pointer() -> Option<CaretAnchor> {
    let mut point = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut point) } == 0 {
        return None;
    }
    Some(CaretAnchor::new(
        point.x as f32,
        point.y as f32,
        1.0,
        CaretSource::PointerFallback,
    ))
}

fn locate_via_foreground_window() -> Option<CaretAnchor> {
    let window = unsafe { GetForegroundWindow() };
    if window.is_null() {
        return None;
    }

    let mut rect = unsafe { zeroed() };
    if unsafe { GetWindowRect(window, &mut rect) } == 0 {
        return None;
    }

    Some(CaretAnchor::new(
        (rect.left + 24) as f32,
        (rect.top + 48) as f32,
        1.0,
        CaretSource::ForegroundWindowFallback,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupPlacement {
    pub x: f32,
    pub y: f32,
}

pub fn popup_placement(anchor: CaretAnchor, width: f32, height: f32, gap: f32) -> PopupPlacement {
    let point = POINT {
        x: anchor.x.round() as i32,
        y: anchor.y.round() as i32,
    };
    let monitor = unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) };
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    let has_monitor = !monitor.is_null() && unsafe { GetMonitorInfoW(monitor, &mut info) } != 0;

    let preferred_y = anchor.y + anchor.height + gap;
    if !has_monitor {
        return PopupPlacement {
            x: anchor.x,
            y: preferred_y,
        };
    }

    let left = info.rcWork.left as f32;
    let top = info.rcWork.top as f32;
    let right = info.rcWork.right as f32;
    let bottom = info.rcWork.bottom as f32;
    let x = anchor.x.clamp(left, (right - width).max(left));
    let y = if preferred_y + height <= bottom {
        preferred_y
    } else {
        (anchor.y - gap - height).max(top)
    };
    PopupPlacement { x, y }
}

#[cfg(test)]
mod tests {
    use super::{CaretAnchor, CaretSource};

    #[test]
    fn caret_anchor_exposes_typed_source_and_geometry() {
        let anchor = CaretAnchor::new(10.0, 20.0, 18.0, CaretSource::UiAutomationCaret);
        assert_eq!(anchor.x(), 10.0);
        assert_eq!(anchor.y(), 20.0);
        assert_eq!(anchor.height(), 18.0);
        assert_eq!(anchor.source(), CaretSource::UiAutomationCaret);
    }
}
