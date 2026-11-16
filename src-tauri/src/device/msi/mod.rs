//! MSI MAG Z890 Tomahawk backend (M3) — ARGB fans on the JARGB_V2 headers.
//!
//! The MSI 761 direct protocol is plain HID feature reports: one 290-byte 0x50
//! setup arms direct mode, then one ~727-byte 0x51 packet per header per frame.
//! A dedicated writer thread owns the HID handle; `apply()` only stages and
//! signals, so the effects engine never blocks on USB.
//!
//! Keeping control is the hard part. The board can lose direct mode at any time
//! — monitor sleep, S3 resume, or another program (MSI's LED keeper) re-applying
//! its own effect — and when it does, 0x51 writes still *succeed* and are simply
//! ignored. So the writer runs a heartbeat that reads the live setup back
//! (`direct_mode_active`) and re-arms the moment it differs, plus a reconnect
//! path for when Windows re-enumerates the controller and the handle dies. The
//! device object itself is never torn down for this; it heals in place.
//!
//! Onboard board RGB is out of scope here; only JARGB_V2_1/2/3 are driven,
//! mapped to MSI header indices 0, 1, and 2.

pub mod controller;
pub mod watchdog;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};

use controller::MsiController;

use super::types::{Color, DeviceInfo, DeviceType, EffectConfig, ZoneInfo};
use super::{DeviceError, RgbDevice};

/// Default LED count per header until the user resizes to the real fan chain.
const DEFAULT_LEDS: u32 = 16;

/// Stable device id (also used by the manager to recognise a known board).
pub fn device_id() -> String {
    format!("msi-{:04x}-{:04x}", controller::VID, controller::PID)
}

/// (zone id, display name, MSI header index hdr1).
const HEADERS: [(&str, &str, u8); 3] = [
    ("jargb_v2_1", "JARGB_V2 1", 0),
    ("jargb_v2_2", "JARGB_V2 2", 1),
    ("jargb_v2_3", "JARGB_V2 3", 2),
];

pub struct MsiDevice {
    info: DeviceInfo,
    shared: Arc<Shared>,
    /// zone id -> staged colors (committed to hardware on apply()).
    staged: HashMap<String, Vec<Color>>,
    worker: Option<JoinHandle<()>>,
}

struct WorkerState {
    frame: HashMap<String, Vec<Color>>,
    generation: u64,
    stop: bool,
    /// Set by `resync()` (wake / display-on / conflict cleanup): re-send the
    /// direct-mode setup and the current frame on the next pass, no questions.
    force_rearm: bool,
}

struct Shared {
    state: Mutex<WorkerState>,
    cond: Condvar,
}

/// Tracks the writer thread's single in-flight HID feature-report call so the
/// watchdog can abort it if the MCU drops the report (see `watchdog.rs`).
///
/// `cur_io_seq` is 0 between I/Os and a unique non-zero id while one is pending;
/// `cur_io_start_ms` is when that I/O began (ms since `base`). The watchdog
/// cancels once per `seq` so it never double-fires on the same call.
struct WriteGuard {
    base: Instant,
    cur_io_seq: AtomicU64,
    cur_io_start_ms: AtomicU64,
    next_seq: AtomicU64,
    writer_thread: AtomicIsize,
    stop: AtomicBool,
    /// The watchdog thread, so `io_begin` can wake it from its idle park.
    watchdog: OnceLock<thread::Thread>,
}

impl WriteGuard {
    fn new() -> Self {
        WriteGuard {
            base: Instant::now(),
            cur_io_seq: AtomicU64::new(0),
            cur_io_start_ms: AtomicU64::new(0),
            next_seq: AtomicU64::new(0),
            writer_thread: AtomicIsize::new(0),
            stop: AtomicBool::new(false),
            watchdog: OnceLock::new(),
        }
    }

    /// Mark a HID I/O as starting. Store start time before the seq so the
    /// watchdog never sees a fresh seq paired with a stale start time.
    fn io_begin(&self) {
        self.cur_io_start_ms
            .store(self.base.elapsed().as_millis() as u64, Ordering::Relaxed);
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed) + 1;
        self.cur_io_seq.store(seq, Ordering::Release);
        if let Some(t) = self.watchdog.get() {
            t.unpark();
        }
    }

    fn io_end(&self) {
        self.cur_io_seq.store(0, Ordering::Release);
    }
}

/// How long a single HID feature-report write may block before the watchdog
/// aborts it. Normal writes are ~12ms; a dropped report otherwise blocks for the
/// ~5s Windows HID timeout.
const WRITE_TIMEOUT_MS: u64 = 150;

/// Watchdog loop: abort the writer's in-flight HID I/O once it has been pending
/// past `WRITE_TIMEOUT_MS`, at most once per I/O. Parks while no I/O is in
/// flight (woken by `io_begin`), so an idle board costs no periodic wakeups.
fn watchdog_loop(guard: Arc<WriteGuard>) {
    let mut last_cancelled = 0u64;
    loop {
        if guard.stop.load(Ordering::Relaxed) {
            return;
        }
        let seq = guard.cur_io_seq.load(Ordering::Acquire);
        if seq == 0 || seq == last_cancelled {
            thread::park_timeout(Duration::from_secs(1));
            continue;
        }
        thread::sleep(Duration::from_millis(25));
        if guard.cur_io_seq.load(Ordering::Acquire) != seq {
            continue; // finished in time
        }
        let now = guard.base.elapsed().as_millis() as u64;
        let started = guard.cur_io_start_ms.load(Ordering::Relaxed);
        if now.saturating_sub(started) >= WRITE_TIMEOUT_MS {
            let h = guard.writer_thread.load(Ordering::Relaxed);
            watchdog::cancel_sync_io(h);
            last_cancelled = seq;
            crate::klog!(
                "msi-watchdog: HID write stuck >{}ms (seq {}), cancelled sync I/O",
                WRITE_TIMEOUT_MS,
                seq
            );
        }
    }
}

/// Detect the board. Ok(None) = not present.
pub fn probe() -> Result<Option<MsiDevice>, DeviceError> {
    let Some(ctl) = MsiController::open()? else {
        return Ok(None);
    };
    // Arm direct mode on the JARGB headers before the first frame.
    ctl.send_setup()?;

    let zones: Vec<ZoneInfo> = HEADERS
        .iter()
        .map(|(id, name, _)| ZoneInfo {
            id: id.to_string(),
            name: name.to_string(),
            led_count: DEFAULT_LEDS,
            resizable: true,
            min_leds: 1,
            max_leds: controller::MAX_LEDS as u32,
            keys: None,
        })
        .collect();

    let staged: HashMap<String, Vec<Color>> = zones
        .iter()
        .map(|z| (z.id.clone(), vec![Color::BLACK; z.led_count as usize]))
        .collect();

    let info = DeviceInfo {
        id: device_id(),
        name: "MSI MAG Z890 Tomahawk".to_string(),
        device_type: DeviceType::Motherboard,
        zones,
        supported_effects: EffectConfig::ALL_KINDS.map(String::from).to_vec(),
    };

    let shared = Arc::new(Shared {
        state: Mutex::new(WorkerState {
            frame: staged.clone(),
            generation: 0,
            stop: false,
            force_rearm: false,
        }),
        cond: Condvar::new(),
    });

    let worker_shared = shared.clone();
    let worker = thread::Builder::new()
        .name("msi-writer".to_string())
        .spawn(move || worker(ctl, worker_shared))
        .map_err(|e| DeviceError::Comm(e.to_string()))?;

    Ok(Some(MsiDevice {
        info,
        shared,
        staged,
        worker: Some(worker),
    }))
}

/// Minimum spacing between frame writes (~20 fps). The engine produces ~30 fps;
/// the MSI MCU drops feature reports when fed that fast (a dropped report blocks
/// the SetFeature call for the full ~5s Windows HID timeout, freezing the fans).
/// Capping the write rate cuts the report volume and, via the latest-generation
/// grab below, coalesces to the freshest frame so motion stays smooth.
const MIN_FRAME_INTERVAL: Duration = Duration::from_millis(50);

/// After this many consecutive failed frames, start trying to re-open the
/// controller. Sleep / monitor-off can make Windows re-enumerate the device,
/// which kills the old HID handle for good — no write on it will ever succeed
/// again, so re-arming direct mode isn't enough; only a fresh open recovers.
const RECONNECT_AFTER_FAILURES: u32 = 3;

/// Idle heartbeat. Every beat the writer reads the board's live setup back; if
/// it left direct mode (firmware reset after sleep, or other software took it)
/// the setup is re-sent. Then the current frame is rewritten, which also keeps
/// Windows from selective-suspending the controller while the display is off.
/// Cost: one 290-byte read + three 727-byte writes every 3s.
const HEARTBEAT: Duration = Duration::from_secs(3);

/// Back-off between retries while the controller is unreachable: quick at
/// first, then capped so a long-absent board isn't hammered with enumerations.
fn retry_delay(failures: u32) -> Duration {
    Duration::from_millis(match failures {
        0..=3 => 500,
        4..=10 => 2_000,
        _ => 5_000,
    })
}

/// Log the first few failures of a streak, then every 50th, so an unplugged or
/// dead controller can't flood the log file.
fn should_log(failures: u32) -> bool {
    failures <= 3 || failures.is_multiple_of(50)
}

fn worker(mut ctl: MsiController, shared: Arc<Shared>) {
    let mut seen_generation = 0u64;
    let mut pending_retry = false;
    let mut consecutive_failures = 0u32;
    let mut last_write = Instant::now();
    // Read-back is trusted until a re-arm fails to make it match (some future
    // firmware might normalise bytes); then we fall back to blind rewrites
    // rather than re-arming every heartbeat.
    let mut readback_trusted = true;
    let mut last_verify = Instant::now();

    // Spawn the watchdog that aborts a HID write the MCU stalls on (~5s freeze).
    let guard = Arc::new(WriteGuard::new());
    guard
        .writer_thread
        .store(watchdog::current_thread_handle(), Ordering::Relaxed);
    {
        let wg = guard.clone();
        if let Ok(h) = thread::Builder::new()
            .name("msi-watchdog".to_string())
            .spawn(move || watchdog_loop(wg))
        {
            let _ = guard.watchdog.set(h.thread().clone());
        }
    }

    loop {
        // Block until the engine has a newer frame, a resync was requested, we
        // owe a retry, or the heartbeat is due.
        let (heartbeat, rearm) = {
            let mut st = shared.state.lock();
            let mut heartbeat = false;
            while !st.stop && st.generation == seen_generation && !pending_retry && !st.force_rearm
            {
                if shared.cond.wait_for(&mut st, HEARTBEAT).timed_out() {
                    heartbeat = true;
                    break;
                }
            }
            if st.stop {
                guard.stop.store(true, Ordering::Relaxed);
                if let Some(t) = guard.watchdog.get() {
                    t.unpark();
                }
                watchdog::close_handle(guard.writer_thread.load(Ordering::Relaxed));
                return;
            }
            (heartbeat, std::mem::take(&mut st.force_rearm))
        };

        // Rate cap: keep writes >= MIN_FRAME_INTERVAL apart. The engine keeps
        // advancing generations during this sleep; we grab the freshest one
        // afterwards, so intermediate frames coalesce instead of queueing.
        let since = last_write.elapsed();
        if since < MIN_FRAME_INTERVAL {
            thread::sleep(MIN_FRAME_INTERVAL - since);
        }

        let frame = {
            let st = shared.state.lock();
            seen_generation = st.generation;
            st.frame.clone()
        };

        last_write = Instant::now();
        let result = (|| -> Result<(), DeviceError> {
            if rearm {
                crate::klog!("msi: resync requested — re-arming direct mode");
                guarded(&guard, || ctl.send_setup())?;
            } else if (heartbeat || last_verify.elapsed() >= HEARTBEAT)
                && readback_trusted
                && {
                    // Time-based, not just on idle timeouts: an animated
                    // effect keeps the condvar busy and would otherwise never
                    // get its direct-mode check.
                    last_verify = Instant::now();
                    !guarded(&guard, || ctl.direct_mode_active())?
                }
            {
                crate::klog!(
                    "msi: board left direct mode (sleep/firmware reset or other RGB software) — re-arming"
                );
                guarded(&guard, || ctl.send_setup())?;
                if !guarded(&guard, || ctl.direct_mode_active())? {
                    readback_trusted = false;
                    crate::klog!(
                        "msi: setup read-back still differs after re-arm; disabling read-back checks"
                    );
                }
            }
            write_frame(&ctl, &guard, &frame).map(|_| ())
        })();

        match result {
            Ok(()) => {
                if consecutive_failures > 0 {
                    crate::klog!("msi: recovered after {consecutive_failures} failed attempts");
                }
                pending_retry = false;
                consecutive_failures = 0;
            }
            Err(e) => {
                pending_retry = true;
                consecutive_failures += 1;
                if should_log(consecutive_failures) {
                    crate::klog!("msi: frame write failed ({consecutive_failures} in a row): {e}");
                }
                if consecutive_failures >= RECONNECT_AFTER_FAILURES {
                    match MsiController::open() {
                        Ok(Some(new_ctl)) => match guarded(&guard, || new_ctl.send_setup()) {
                            Ok(()) => {
                                crate::klog!(
                                    "msi: controller re-opened after {consecutive_failures} failed frames"
                                );
                                ctl = new_ctl;
                                readback_trusted = true;
                                // Next pass rewrites the latest frame at once.
                                continue;
                            }
                            Err(e) => {
                                if should_log(consecutive_failures) {
                                    crate::klog!("msi: reconnect setup failed: {e}")
                                }
                            }
                        },
                        Ok(None) => {
                            if should_log(consecutive_failures) {
                                crate::klog!("msi: reconnect: controller not enumerated (yet)")
                            }
                        }
                        Err(e) => {
                            if should_log(consecutive_failures) {
                                crate::klog!("msi: reconnect failed: {e}")
                            }
                        }
                    }
                }
                // Sleep on the condvar so a stop/resync still lands promptly.
                let mut st = shared.state.lock();
                if !st.stop && !st.force_rearm {
                    shared
                        .cond
                        .wait_for(&mut st, retry_delay(consecutive_failures));
                }
            }
        }
    }
}

/// Spacing inserted between consecutive per-header feature reports within a
/// frame. The MCU has a single report buffer and drops a report that lands
/// before it has drained the previous one; a few ms lets it catch up. Matches
/// the pacing OpenRGB applies to this controller.
const INTER_PACKET_GAP: Duration = Duration::from_millis(2);

/// Write every staged header for one frame. Returns the number of headers that
/// had to re-arm direct mode to succeed (0 = clean frame). Headers are spaced
/// `INTER_PACKET_GAP` apart so the MCU doesn't drop a report.
fn write_frame(
    ctl: &MsiController,
    guard: &WriteGuard,
    frame: &HashMap<String, Vec<Color>>,
) -> Result<u32, DeviceError> {
    let mut rearms = 0;
    let mut first = true;
    for (id, _, hdr1) in HEADERS {
        if let Some(colors) = frame.get(id) {
            if !first {
                thread::sleep(INTER_PACKET_GAP);
            }
            first = false;
            if write_header(ctl, guard, hdr1, colors)? {
                rearms += 1;
            }
        }
    }
    Ok(rearms)
}

/// Run one HID feature-report call bracketed by the watchdog guard so a stalled
/// call can be aborted independently of the others in the frame.
fn guarded<T>(
    guard: &WriteGuard,
    f: impl FnOnce() -> Result<T, DeviceError>,
) -> Result<T, DeviceError> {
    guard.io_begin();
    let r = f();
    guard.io_end();
    r
}

/// Write a header, re-arming direct mode and retrying once on failure (the
/// board can drop direct mode after an error). Returns `Ok(true)` if the first
/// write failed and a re-arm was needed to recover, `Ok(false)` if it wrote
/// cleanly on the first try.
fn write_header(
    ctl: &MsiController,
    guard: &WriteGuard,
    hdr1: u8,
    colors: &[Color],
) -> Result<bool, DeviceError> {
    if guarded(guard, || ctl.set_header(hdr1, colors)).is_ok() {
        return Ok(false);
    }
    guarded(guard, || ctl.send_setup())?;
    guarded(guard, || ctl.set_header(hdr1, colors))?;
    Ok(true)
}

impl RgbDevice for MsiDevice {
    fn id(&self) -> &str {
        &self.info.id
    }

    fn info(&self) -> &DeviceInfo {
        &self.info
    }

    fn set_zone_leds(&mut self, zone_id: &str, colors: &[Color]) -> Result<(), DeviceError> {
        let staged = self
            .staged
            .get_mut(zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        let n = staged.len().min(colors.len());
        staged[..n].copy_from_slice(&colors[..n]);
        Ok(())
    }

    fn apply(&mut self) -> Result<(), DeviceError> {
        let mut st = self.shared.state.lock();
        st.frame.clone_from(&self.staged);
        st.generation += 1;
        drop(st);
        self.shared.cond.notify_one();
        Ok(())
    }

    fn resize_zone(&mut self, zone_id: &str, led_count: u32) -> Result<(), DeviceError> {
        let zone = self
            .info
            .zones
            .iter_mut()
            .find(|z| z.id == zone_id)
            .ok_or_else(|| DeviceError::UnknownZone(zone_id.to_string()))?;
        if !zone.resizable {
            return Err(DeviceError::NotResizable(zone_id.to_string()));
        }
        if led_count < zone.min_leds || led_count > zone.max_leds {
            return Err(DeviceError::LedCountOutOfRange {
                count: led_count,
                min: zone.min_leds,
                max: zone.max_leds,
            });
        }
        zone.led_count = led_count;
        self.staged
            .insert(zone_id.to_string(), vec![Color::BLACK; led_count as usize]);
        Ok(())
    }

    fn resync(&mut self) {
        self.shared.state.lock().force_rearm = true;
        self.shared.cond.notify_one();
    }
}

impl Drop for MsiDevice {
    fn drop(&mut self) {
        {
            let mut st = self.shared.state.lock();
            st.stop = true;
        }
        self.shared.cond.notify_one();
        // Wait for the writer to let go of the HID handle, so a re-probe right
        // after this never has two writers on the board at once. Bounded by the
        // HID write timeout in the worst case.
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}
