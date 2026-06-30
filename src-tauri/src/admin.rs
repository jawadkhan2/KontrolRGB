//! Windows UAC elevation helpers.
//!
//! Two jobs: report whether the current process is already elevated, and
//! relaunch the same executable with the "runas" verb (which pops the UAC
//! prompt). The settings "ask to run as administrator on startup" toggle uses
//! both — at startup the frontend checks `is_elevated`, and if the toggle is on
//! and we're not elevated it calls `relaunch_as_admin`.

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
#[cfg(windows)]
pub fn relaunch_as_admin() -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    const SW_SHOWNORMAL: i32 = 1;

    let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;

    let wide: Vec<u16> = exe.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let verb: Vec<u16> = "runas".encode_utf16().chain(std::iter::once(0)).collect();

    // ShellExecuteW returns an HINSTANCE; a value <= 32 means failure (e.g.
    // SE_ERR_ACCESSDENIED when the user clicks "No" on the UAC dialog).
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            wide.as_ptr(),
            std::ptr::null(),
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
