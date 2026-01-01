//! Input event handling for WPE WebView.
//!
//! This module provides types and utilities for converting platform input events
//! (from winit or other sources) into WPE input events.

use std::time::{SystemTime, UNIX_EPOCH};

/// Mouse button identifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    Back,
    Forward,
    Other(u32),
}

impl MouseButton {
    /// Convert to WPE button number (1-indexed).
    #[must_use]
    pub fn to_wpe_button(self) -> u32 {
        match self {
            Self::Left => 1,
            Self::Middle => 2,
            Self::Right => 3,
            Self::Back => 8,
            Self::Forward => 9,
            Self::Other(n) => n,
        }
    }
}

/// Keyboard modifier flags.
#[derive(Debug, Clone, Copy, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub meta: bool,
    pub caps_lock: bool,
}

impl Modifiers {
    /// Convert to WPE modifier flags.
    #[must_use]
    pub fn to_wpe_modifiers(self) -> u32 {
        let mut flags = 0u32;
        if self.ctrl {
            flags |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_CONTROL;
        }
        if self.shift {
            flags |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_SHIFT;
        }
        if self.alt {
            flags |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_ALT;
        }
        if self.meta {
            flags |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_META;
        }
        if self.caps_lock {
            flags |= wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_CAPS_LOCK;
        }
        flags
    }
}

/// Get current timestamp in milliseconds.
#[must_use]
pub fn current_time_ms() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| (d.as_millis() & 0xFFFF_FFFF) as u32)
        .unwrap_or(0)
}

/// Input source type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputSource {
    Mouse,
    Touchpad,
    Touchscreen,
    Pen,
    Keyboard,
}

impl InputSource {
    /// Convert to WPE input source.
    #[must_use]
    pub fn to_wpe_source(self) -> u32 {
        match self {
            Self::Mouse => wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE,
            Self::Touchpad => wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_TOUCHPAD,
            Self::Touchscreen => wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_TOUCHSCREEN,
            Self::Pen => wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_PEN,
            Self::Keyboard => wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_KEYBOARD,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mouse_button_to_wpe() {
        assert_eq!(MouseButton::Left.to_wpe_button(), 1);
        assert_eq!(MouseButton::Middle.to_wpe_button(), 2);
        assert_eq!(MouseButton::Right.to_wpe_button(), 3);
        assert_eq!(MouseButton::Back.to_wpe_button(), 8);
        assert_eq!(MouseButton::Forward.to_wpe_button(), 9);
        assert_eq!(MouseButton::Other(42).to_wpe_button(), 42);
    }

    #[test]
    fn test_modifiers_to_wpe() {
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            alt: false,
            meta: false,
            caps_lock: false,
        };
        let wpe_mods = mods.to_wpe_modifiers();
        assert!(wpe_mods & wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_CONTROL != 0);
        assert!(wpe_mods & wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_SHIFT != 0);
        assert!(wpe_mods & wpe_sys::WPEModifiers_WPE_MODIFIER_KEYBOARD_ALT == 0);
    }

    #[test]
    fn test_modifiers_default() {
        let mods = Modifiers::default();
        assert!(!mods.ctrl);
        assert!(!mods.shift);
        assert!(!mods.alt);
        assert!(!mods.meta);
        assert!(!mods.caps_lock);
        assert_eq!(mods.to_wpe_modifiers(), 0);
    }

    #[test]
    fn test_current_time_ms() {
        let time1 = current_time_ms();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let time2 = current_time_ms();
        // Time should advance (allowing for some tolerance)
        assert!(time2 >= time1);
    }

    #[test]
    fn test_input_source_to_wpe() {
        assert_eq!(InputSource::Mouse.to_wpe_source(), wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_MOUSE);
        assert_eq!(InputSource::Keyboard.to_wpe_source(), wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_KEYBOARD);
        assert_eq!(InputSource::Touchpad.to_wpe_source(), wpe_sys::WPEInputSource_WPE_INPUT_SOURCE_TOUCHPAD);
    }
}
