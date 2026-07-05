//! Wake-from-sleep watcher (Windows only). Monitor sleep and system suspend
//! can reset or re-enumerate the USB RGB controllers: the MSI Mystic Light MCU
//! drops direct mode and reverts to its firmware rainbow, and its HID handle
//! can go permanently dead. Nothing else in the app notices — the effects
//! engine just keeps (not) writing. So this module runs a hidden window whose
//! only job is to receive WM_POWERBROADCAST (system resume + console display
//! state changes) and fire a debounced callback that re-probes the hardware.
//!
//! A *hidden top-level* window is required: message-only (HWND_MESSAGE)
//! windows never receive broadcast messages, so they'd sit silent here.

use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// Channel from the window proc (message thread) to the recovery thread.
static WAKE_TX: OnceLock<Sender<()>> = OnceLock::new();

/// Debounce: one wake produces a burst of power messages (resume + display-on
/// + dim transitions); wait for this much silence before acting on them.
const QUIET: Duration = Duration::from_secs(1);
/// Grace period before recovery runs, giving Windows time to re-enumerate the
/// USB devices it suspended.
const SETTLE: Duration = Duration::from_millis(2500);
/// Minimum spacing between recovery runs, so flappy display events (screen
/// dimming on/off repeatedly) can't rescan in a loop.
const MIN_GAP: Duration = Duration::from_secs(10);

/// Start watching. `on_wake` runs on a dedicated thread after each debounced
/// wake event; it may block (the recovery path sleeps and re-probes).
pub fn spawn(on_wake: impl Fn() + Send + 'static) {
    let (tx, rx) = mpsc::channel::<()>();
    if WAKE_TX.set(tx).is_err() {
        return; // already watching
    }

    if let Err(e) = thread::Builder::new()
        .name("power-watch-msg".into())
        .spawn(message_loop)
    {
        eprintln!("power-watch: failed to spawn message thread: {e}");
        return;
    }

    let _ = thread::Builder::new()
        .name("power-watch-recover".into())
        .spawn(move || {
            let mut last_run: Option<Instant> = None;
            while rx.recv().is_ok() {
                // Coalesce the burst until QUIET of silence.
                while rx.recv_timeout(QUIET).is_ok() {}
                if last_run.is_some_and(|t| t.elapsed() < MIN_GAP) {
                    continue;
                }
                thread::sleep(SETTLE);
                // Drop anything that queued while settling — it's the same wake.
                while rx.try_recv().is_ok() {}
                last_run = Some(Instant::now());
                on_wake();
            }
        });
}

/// Called from the window proc on any wake-ish power event.
fn notify_wake() {
    if let Some(tx) = WAKE_TX.get() {
        let _ = tx.send(());
    }
}

fn message_loop() {
    use windows_sys::core::GUID;
    use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::System::Power::{
        RegisterPowerSettingNotification, POWERBROADCAST_SETTING,
    };
    use windows_sys::Win32::System::SystemServices::GUID_CONSOLE_DISPLAY_STATE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, RegisterClassW,
        TranslateMessage, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
        PBT_POWERSETTINGCHANGE, WM_POWERBROADCAST, WNDCLASSW,
    };

    fn guid_eq(a: &GUID, b: &GUID) -> bool {
        a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
    }

    unsafe extern "system" fn wnd_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        if msg == WM_POWERBROADCAST {
            match wparam as u32 {
                // System resume (fires with or without a logged-in user).
                PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND => notify_wake(),
                PBT_POWERSETTINGCHANGE => {
                    let setting = unsafe { &*(lparam as *const POWERBROADCAST_SETTING) };
                    // CONSOLE_DISPLAY_STATE data: 0 = off, 1 = on, 2 = dimmed.
                    // Only display-ON matters — that's the moment the user is
                    // back and the USB devices are (re)waking.
                    if guid_eq(&setting.PowerSetting, &GUID_CONSOLE_DISPLAY_STATE)
                        && setting.DataLength >= 1
                        && setting.Data[0] == 1
                    {
                        notify_wake();
                    }
                }
                _ => {}
            }
            return 1; // TRUE
        }
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    let class_name: Vec<u16> = "KontrolRGBPowerWatch\0".encode_utf16().collect();
    unsafe {
        let hinstance = GetModuleHandleW(std::ptr::null());
        let mut wc: WNDCLASSW = std::mem::zeroed();
        wc.lpfnWndProc = Some(wnd_proc);
        wc.hInstance = hinstance;
        wc.lpszClassName = class_name.as_ptr();
        if RegisterClassW(&wc) == 0 {
            eprintln!("power-watch: RegisterClassW failed");
            return;
        }

        let hwnd = CreateWindowExW(
            0,
            class_name.as_ptr(),
            class_name.as_ptr(),
            0, // no WS_VISIBLE — never shown
            0,
            0,
            0,
            0,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinstance,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            eprintln!("power-watch: CreateWindowExW failed");
            return;
        }

        // Resume events broadcast to every top-level window automatically, but
        // display on/off does not — subscribe explicitly.
        // Flag 0 = DEVICE_NOTIFY_WINDOW_HANDLE.
        // HPOWERNOTIFY is an isize handle in windows-sys; 0 = failure.
        let reg = RegisterPowerSettingNotification(hwnd, &GUID_CONSOLE_DISPLAY_STATE, 0);
        if reg == 0 {
            eprintln!("power-watch: RegisterPowerSettingNotification failed");
            // Keep running: plain suspend/resume broadcasts still arrive.
        }

        let mut msg: MSG = std::mem::zeroed();
        loop {
            let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
            if r <= 0 {
                return;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
