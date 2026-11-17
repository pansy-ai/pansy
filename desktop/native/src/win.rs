//! The Windows calls the desktop needs: one window per session, and the
//! console of the terminal `pansy <command>` was started from.
#![allow(unsafe_code)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;

use windows_sys::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_OUTPUT_HANDLE,
};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowW, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
};

fn wide(text: impl AsRef<OsStr>) -> Vec<u16> {
    text.as_ref().encode_wide().chain(Some(0)).collect()
}

/// Claims the session's one window; false when another instance has it, in
/// which case that instance's window (titled `title`) is brought forward.
pub fn claim_window(title: &str) -> bool {
    let name = wide(r"Local\dev.pansy.app.window");
    // SAFETY: a valid name. The mutex lives as long as the process: leaked
    // on purpose.
    let mutex = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    // SAFETY: no preconditions.
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    if mutex.is_null() || !existed {
        return true;
    }
    let title = wide(title);
    // SAFETY: a valid title; a null window means none was found.
    let window = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if !window.is_null() {
        // SAFETY: a window handle FindWindowW returned.
        unsafe {
            if IsIconic(window) != 0 {
                ShowWindow(window, SW_RESTORE);
            }
            SetForegroundWindow(window);
        }
    }
    false
}

/// A windowed program has no console: print to the terminal it was started
/// from, unless its output already goes somewhere (a pipe or a file).
pub fn attach_parent_console() {
    // SAFETY: no pointers.
    unsafe {
        if GetStdHandle(STD_OUTPUT_HANDLE).is_null() {
            AttachConsole(ATTACH_PARENT_PROCESS);
        }
    }
}
