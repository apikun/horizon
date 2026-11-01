//! One line into Android's log buffer: the safe wrapper over
//! `__android_log_write` that the game's log (horizon-client-render, which
//! forbids unsafe) calls, so `adb logcat -s Horizon` mirrors `latest.log` on
//! a phone, which has no terminal. On the desktop every function is a no-op.

/// `android/log.h`'s priorities, the ones the game's log uses.
pub const INFO: i32 = 4;
pub const WARN: i32 = 5;
pub const ERROR: i32 = 6;

/// Writes one line under `tag`. Best effort: a failed write says nothing.
#[cfg(target_os = "android")]
pub fn write(priority: i32, tag: &str, message: &str) {
    use std::ffi::CString;
    let Ok(tag) = CString::new(tag) else { return };
    // NUL bytes would cut the line short; logcat shows the rest badly anyway.
    let Ok(message) = CString::new(message.replace('\0', " ")) else {
        return;
    };
    // SAFETY: both arguments are valid NUL-terminated C strings, alive for
    // the duration of the call.
    unsafe {
        android_log_sys::__android_log_write(priority, tag.as_ptr(), message.as_ptr());
    }
}

#[cfg(not(target_os = "android"))]
pub fn write(_priority: i32, _tag: &str, _message: &str) {}
