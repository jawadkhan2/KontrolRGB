//! Thread-level I/O cancellation for the MSI writer.
//!
//! The Mystic Light MCU sporadically drops a 727-byte HID feature report when
//! streamed back-to-back. When that happens the synchronous `HidD_SetFeature`
//! (inside hidapi's `send_feature_report`) blocks for the *full* Windows HID
//! class-driver timeout (~5s), freezing the fans on their last color until it
//! finally errors out. Rate-limiting/pacing the writes does not stop the drops.
//!
//! Instead we let a watchdog thread abort the stuck call: it holds a real handle
//! to the writer thread and calls `CancelSynchronousIo` once a single HID I/O
//! has been pending past a short deadline. That makes `send_feature_report`
//! return `ERROR_OPERATION_ABORTED` promptly; the writer's existing re-arm +
//! retry then recovers. A 5s freeze collapses to a ~deadline-sized hiccup.

/// Duplicate the *current* thread's pseudo-handle into a real handle usable from
/// another thread (the pseudo-handle from `GetCurrentThread` only resolves in
/// the thread that called it). Returns 0 on failure. Call from the writer thread.
#[cfg(windows)]
pub fn current_thread_handle() -> isize {
    use windows_sys::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};

    unsafe {
        let mut real: HANDLE = std::ptr::null_mut();
        let proc = GetCurrentProcess();
        let ok = DuplicateHandle(
            proc,
            GetCurrentThread(),
            proc,
            &mut real,
            0,
            0,
            DUPLICATE_SAME_ACCESS,
        );
        if ok == 0 {
            0
        } else {
            real as isize
        }
    }
}

/// Cancel any synchronous I/O currently pending on the given thread handle.
/// No-op (returns) if nothing is pending, so it's safe to call speculatively.
#[cfg(windows)]
pub fn cancel_sync_io(handle: isize) {
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::IO::CancelSynchronousIo;
    if handle == 0 {
        return;
    }
    unsafe {
        CancelSynchronousIo(handle as HANDLE);
    }
}

/// Close a handle obtained from [`current_thread_handle`].
#[cfg(windows)]
pub fn close_handle(handle: isize) {
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    if handle == 0 {
        return;
    }
    unsafe {
        CloseHandle(handle as HANDLE);
    }
}

#[cfg(not(windows))]
pub fn current_thread_handle() -> isize {
    0
}

#[cfg(not(windows))]
pub fn cancel_sync_io(_handle: isize) {}

#[cfg(not(windows))]
pub fn close_handle(_handle: isize) {}
