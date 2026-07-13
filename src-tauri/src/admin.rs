//! Windows UAC elevation helpers.
//!
//! Two jobs: report whether the current process is already elevated, and
//! relaunch the same executable with the "runas" verb (which pops the UAC
//! prompt). The settings "ask to run as administrator on startup" toggle uses
//! both — at startup the frontend checks `is_elevated`, and if the toggle is on
//! and we're not elevated it calls `relaunch_as_admin`.
//!
//! The relaunch is the one place where two KontrolRGB processes legitimately
//! exist at once, which collides with the single-instance guard: the elevated
//! child would find the old instance's mutex and bounce straight back into it,
//! killing the elevation. So the child is passed `--wait-for-pid=<old pid>` and
//! blocks on it (see `wait_for_parent_exit`, called before the guard is armed).
//! Handing the process over this way — rather than tearing the guard down before
//! elevating — also means a declined UAC prompt leaves the old instance running
//! and still guarded, instead of unprotected.

/// Tells a freshly-launched instance which process it is replacing.
#[cfg(windows)]
const WAIT_FOR_PID_ARG: &str = "--wait-for-pid=";

/// Block until the process we are replacing has exited, so its HID handles, fan
/// driver and single-instance mutex are all released before we claim them. No-op
/// unless we were launched with `--wait-for-pid`. Bounded, so a wedged parent
/// costs us a slow start rather than a hang.
#[cfg(windows)]
pub fn wait_for_parent_exit() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::Storage::FileSystem::SYNCHRONIZE;
    use windows_sys::Win32::System::Threading::{OpenProcess, WaitForSingleObject};

    let Some(pid) = std::env::args().find_map(|arg| {
        arg.strip_prefix(WAIT_FOR_PID_ARG)
            .and_then(|pid| pid.parse::<u32>().ok())
    }) else {
        return;
    };

    unsafe {
        // Null handle = the parent is already gone (or unopenable), which is the
        // outcome we're waiting for anyway.
        let parent = OpenProcess(SYNCHRONIZE, 0, pid);
        if parent.is_null() {
            return;
        }
        WaitForSingleObject(parent, 15_000);
        CloseHandle(parent);
    }
}

#[cfg(not(windows))]
pub fn wait_for_parent_exit() {}

#[cfg(windows)]
pub fn is_elevated() -> bool {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }

        let mut elevation = TOKEN_ELEVATION {
            TokenIsElevated: 0,
        };
        let mut ret_len = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut ret_len,
        );
        CloseHandle(token);

        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// Relaunch this executable elevated via ShellExecuteW("runas"). Returns Ok on
/// success (the UAC-approved elevated process is starting and the caller should
/// exit this one); Err if the path can't be resolved or the user declines UAC.
///
/// The child is told to wait on our PID, so it stays parked outside the
/// single-instance guard — and off the hardware — until we're actually gone.
#[cfg(windows)]
pub fn relaunch_as_admin() -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    const SW_SHOWNORMAL: i32 = 1;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;

    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();
    let args: Vec<u16> = format!("{WAIT_FOR_PID_ARG}{}", std::process::id())
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // ShellExecuteW returns an HINSTANCE; a value <= 32 means failure (e.g.
    // SE_ERR_ACCESSDENIED when the user clicks "No" on the UAC dialog).
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            args.as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };

    if result as isize <= 32 {
        return Err("elevation cancelled or failed".into());
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn is_elevated() -> bool {
    true
}

#[cfg(not(windows))]
pub fn relaunch_as_admin() -> Result<(), String> {
    Err("elevation only supported on Windows".into())
}
