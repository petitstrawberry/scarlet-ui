//! Thread-affine C access to the standard ScarletUI Window frame.
//! A host retains window/event-loop ownership and composes its external image
//! with the borrowed chrome pixels. See include/scarlet_ui_window.h.
#[cfg(target_os = "scarlet")]
core::arch::global_asm!(".hidden __scarlet_getauxval", ".hidden __scarlet_start");

use scarlet_ui_core::{
    event::{MouseButton, MouseEvent, WindowEvent},
    window_chrome::{ChromeInsets, ChromeRect, WindowChrome, chrome_insets},
};
use std::{
    ffi::{CStr, c_char, c_void},
    thread::ThreadId,
};

pub struct ScarletUiFrame {
    thread: ThreadId,
    chrome: Option<WindowChrome>,
    config: Option<(String, u32, u32, u32, u32)>,
}
#[repr(C)]
pub struct FrameView {
    width: u32,
    height: u32,
    stride: u32,
    margin: u32,
    insets: ChromeInsets,
    pixels: *const c_void,
    bytes: usize,
    clip: *const ChromeRect,
    clip_count: usize,
}
unsafe fn frame<'a>(ptr: *mut ScarletUiFrame) -> Option<&'a mut ScarletUiFrame> {
    let frame = unsafe { ptr.as_mut() }?;
    (frame.thread == std::thread::current().id()).then_some(frame)
}
#[unsafe(no_mangle)]
pub extern "C" fn scarlet_ui_frame_abi() -> u32 {
    1
}
/// # Safety
/// `out` must point to writable storage for one inset record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_insets(
    scale: u32,
    flags: u32,
    out: *mut ChromeInsets,
) -> i32 {
    if out.is_null() || !(250..=8000).contains(&scale) || flags & !3 != 0 {
        return -1;
    }
    unsafe {
        out.write(chrome_insets(scale, flags & 1 != 0));
    }
    0
}
#[unsafe(no_mangle)]
pub extern "C" fn scarlet_ui_frame_create() -> *mut ScarletUiFrame {
    Box::into_raw(Box::new(ScarletUiFrame {
        thread: std::thread::current().id(),
        chrome: None,
        config: None,
    }))
}
/// # Safety
/// `ptr` is a live frame on its creating thread; `title` is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_configure(
    ptr: *mut ScarletUiFrame,
    title: *const c_char,
    width: u32,
    height: u32,
    scale: u32,
    flags: u32,
) -> i32 {
    let Some(frame) = (unsafe { frame(ptr) }) else {
        return -1;
    };
    if title.is_null() || flags & !3 != 0 {
        return -1;
    }
    let Ok(title) = (unsafe { CStr::from_ptr(title) }).to_str() else {
        return -1;
    };
    let config = (title.to_owned(), width, height, scale, flags);
    if frame.config.as_ref() == Some(&config) {
        return 0;
    }
    let Ok(chrome) = WindowChrome::new(title, width, height, scale, flags & 1 != 0, flags & 2 != 0)
    else {
        return -1;
    };
    frame.chrome = Some(chrome);
    frame.config = Some(config);
    0
}
/// # Safety
/// `ptr` is a live frame on its creating thread; `out` points to writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_render(
    ptr: *mut ScarletUiFrame,
    out: *mut FrameView,
) -> i32 {
    let Some(frame) = (unsafe { frame(ptr) }) else {
        return -1;
    };
    if out.is_null() {
        return -1;
    }
    let Some(chrome) = frame.chrome.as_mut() else {
        return -1;
    };
    let (_, width, height, _, _) = frame.config.as_ref().unwrap();
    let (insets, margin, clip, clip_count) = (
        chrome.insets(),
        chrome.margin(),
        chrome.clip().as_ptr(),
        chrome.clip().len(),
    );
    let Some(buffer) = chrome.render() else {
        return -2;
    };
    unsafe {
        out.write(FrameView {
            width: *width,
            height: *height,
            stride: *width * 4,
            margin,
            insets,
            pixels: buffer.data().as_ptr().cast(),
            bytes: buffer.data().len(),
            clip,
            clip_count,
        });
    }
    0
}
/// # Safety
/// `ptr` is a live frame on its creating thread; `actions` points to writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_pointer(
    ptr: *mut ScarletUiFrame,
    kind: u32,
    x: i32,
    y: i32,
    actions: *mut u32,
) -> i32 {
    let Some(frame) = (unsafe { frame(ptr) }) else {
        return -1;
    };
    if actions.is_null() {
        return -1;
    }
    let Some(chrome) = frame.chrome.as_mut() else {
        return -1;
    };
    let event = match kind {
        0 => MouseEvent::Moved { x, y },
        1 => MouseEvent::ButtonPressed {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        2 => MouseEvent::ButtonReleased {
            button: MouseButton::Left,
            x,
            y,
            click_count: 1,
        },
        _ => return -1,
    };
    let mut flags = 0;
    for action in chrome.mouse(event) {
        flags |= match action {
            WindowEvent::CloseRequested => 1,
            WindowEvent::MoveRequested => 2,
            WindowEvent::MinimizeRequested => 4,
            WindowEvent::MaximizeRequested | WindowEvent::RestoreRequested => 8,
        };
    }
    unsafe {
        actions.write(flags);
    }
    0
}
/// # Safety
/// `ptr` is a live frame on its creating thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_dirty(ptr: *mut ScarletUiFrame) -> i32 {
    let Some(frame) = (unsafe { frame(ptr) }) else {
        return -1;
    };
    i32::from(frame.chrome.as_ref().is_some_and(WindowChrome::has_dirty))
}
/// # Safety
/// `ptr` is null or a live frame on its creating thread; destroy it exactly once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn scarlet_ui_frame_destroy(ptr: *mut ScarletUiFrame) {
    if unsafe { frame(ptr) }.is_some() {
        drop(unsafe { Box::from_raw(ptr) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ffi_reconfiguration_preserves_valid_state_after_invalid_input() {
        unsafe {
            let frame = scarlet_ui_frame_create();
            let title = c"External content";
            assert_eq!(
                scarlet_ui_frame_configure(frame, title.as_ptr(), 324, 276, 1000, 3),
                0
            );
            assert_eq!(
                scarlet_ui_frame_configure(frame, title.as_ptr(), 0, 0, 1000, 3),
                -1
            );
            let mut view = std::mem::MaybeUninit::<FrameView>::uninit();
            assert_eq!(scarlet_ui_frame_render(frame, view.as_mut_ptr()), 0);
            let view = view.assume_init();
            assert_eq!((view.width, view.height), (324, 276));
            assert!(view.bytes >= (view.stride * view.height) as usize);
            assert!(view.clip_count > 0);
            assert_eq!(
                scarlet_ui_frame_configure(frame, title.as_ptr(), 321, 241, 1000, 0),
                0
            );
            let mut view = std::mem::MaybeUninit::<FrameView>::uninit();
            assert_eq!(scarlet_ui_frame_render(frame, view.as_mut_ptr()), 0);
            assert_eq!(view.assume_init().insets, ChromeInsets::default());
            scarlet_ui_frame_destroy(frame);
        }
    }
}
