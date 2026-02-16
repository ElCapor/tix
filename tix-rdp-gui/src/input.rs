//! Local input capture → protocol event conversion.
//!
//! Translates [`WindowEvent`]s from the Win32 message loop into
//! TIX protocol [`MouseEvent`] / [`KeyEvent`] types that can be
//! serialised and sent to the slave.

use tix_core::protocol::screen::{
    KeyAction, KeyEvent, MouseButton, MouseEvent, MouseEventKind,
};

use crate::window::{MouseBtn, WindowEvent};

/// Convert a window event to a protocol input event (if applicable).
pub fn translate_event(
    event: &WindowEvent,
    window_width: u32,
    window_height: u32,
    remote_width: u32,
    remote_height: u32,
) -> Option<InputAction> {
    match event {
        WindowEvent::MouseMove(x, y) => {
            // Scale from window coordinates to remote coordinates.
            let rx = (*x as f64 / window_width as f64 * remote_width as f64) as i32;
            let ry = (*y as f64 / window_height as f64 * remote_height as f64) as i32;
            Some(InputAction::Mouse(MouseEvent {
                x: rx,
                y: ry,
                button: MouseButton::None,
                kind: MouseEventKind::Move,
                scroll_delta: 0,
            }))
        }
        WindowEvent::MouseButton(btn, pressed) => {
            let button = match btn {
                MouseBtn::Left => MouseButton::Left,
                MouseBtn::Right => MouseButton::Right,
                MouseBtn::Middle => MouseButton::Middle,
            };
            let kind = if *pressed {
                MouseEventKind::Press
            } else {
                MouseEventKind::Release
            };
            Some(InputAction::Mouse(MouseEvent {
                x: 0,
                y: 0,
                button,
                kind,
                scroll_delta: 0,
            }))
        }
        WindowEvent::MouseWheel(delta) => {
            Some(InputAction::Mouse(MouseEvent {
                x: 0,
                y: 0,
                button: MouseButton::None,
                kind: MouseEventKind::Scroll,
                scroll_delta: *delta,
            }))
        }
        WindowEvent::Key(vk, scan, pressed) => {
            let action = if *pressed {
                KeyAction::Press
            } else {
                KeyAction::Release
            };
            Some(InputAction::Key(KeyEvent {
                virtual_key: *vk,
                scan_code: *scan,
                action,
                modifiers: 0,
            }))
        }
        WindowEvent::Close | WindowEvent::Resize(..) => None,
    }
}

/// Tagged union of input actions to send to the slave.
pub enum InputAction {
    Mouse(MouseEvent),
    Key(KeyEvent),
}
