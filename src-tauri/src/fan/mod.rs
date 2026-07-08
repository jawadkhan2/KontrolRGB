//! Fan-control subsystem (MSI Z890 case fans) — a sibling of the RGB `device`
//! tree, deliberately separate: fans aren't `RgbDevice`s, and they reach the
//! hardware through ring-0 LPC port I/O (`lpc`), not USB HID.
//!
//! PHASE 1: monitoring only — detect the NCT6687D-R, read live RPM/PWM, and let
//! the user confirm which *tach* channels are their case fans.
//!
//! PHASE 2 (this layer): actual speed control. A confirmed tach channel still
//! doesn't say which *PWM header* drives it (the board reports the two
//! separately), so `map_pwm_header` empirically discovers the header by nudging
//! each one and watching the tach. Once mapped, `set_speed` commands a clamped
//! duty, a watchdog hands the fan back to the BIOS if the UI stops sending
//! heartbeats (crash / freeze / close), and `sweep` measures the real stall
//! point to tighten the safety floor. Every speed write is clamped through
//! `safety::FanLimits` first; STOP (`release_to_bios`) is always available.
//!
//! Windows-only: the whole module is `#[cfg(windows)]` in `lib.rs`.

#[cfg(windows)]
mod lpc;
#[cfg(windows)]
mod nct6687;
pub mod safety;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
#[cfg(windows)]
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

#[cfg(windows)]
use nct6687::Nct6687;
#[cfg(windows)]
pub use nct6687::{ChannelReading, SweepResult, TempReading};
use safety::FanLimits;

use crate::gpu_telemetry::GpuTelemetrySubsystem;

pub const CONTROLLABLE_CHANNEL_COUNT: u8 = 8;
pub const RPM_CHANNEL_COUNT: u8 = 16;
const PUMP_HEADER_INDEX: u8 = 1;

/// Failsafe: if the in-process control loop stops refreshing this heartbeat
/// while we hold manual control, hand the fans back to the BIOS so a hung or
/// panicked controller can't strand a fan at a fixed duty.
///
/// Control now lives in a backend thread (`start_control_loop`) that beats every
/// `CONTROL_INTERVAL` regardless of window state, so a healthy app always pings
/// well inside this window — webview timer throttling (which crippled the old
/// JS-driven loop when minimized) no longer matters. Only a genuinely dead
/// control thread trips this, after which the BIOS safely manages thermals.
#[cfg(windows)]
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(10);

/// How often the backend control loop re-evaluates every mapped fan's target
/// (curve interpolation or manual duty) and writes the chip. Runs on its own OS
/// thread so it is immune to webview timer throttling when the window is hidden.
/// 1s so a temperature spike moves the fans within a second; each pass is a few
/// cheap EC reads plus at most one duty write per fan (2% dedup), so the faster
/// cadence adds negligible EC traffic.
#[cfg(windows)]
const CONTROL_INTERVAL: Duration = Duration::from_secs(1);

/// Thermal failsafe. If ANY chip temperature source (or the GPU die) reaches
/// this, the control loop abandons the user's curve/manual duty AND the noise
/// ceiling and drives every fan it controls to 100% until the temperature
/// recovers. A misconfigured flat curve, a low manual duty, or a `max_pwm`
/// noise cap can otherwise hold fans low into a thermal event — the PWM floor
/// only guards against *stall*, never against *overheat*. This is the airflow
/// backstop that closes that gap.
#[cfg(windows)]
const EMERGENCY_TEMP_C: f32 = 90.0;

/// Once the emergency has engaged, keep forcing 100% until every temperature
/// has dropped this far below the trip point, so a reading hovering right at
/// the threshold can't flap the fans on and off.
#[cfg(windows)]
const EMERGENCY_CLEAR_C: f32 = 85.0;

/// Curve-tracking hysteresis (°C). A curve fan only recomputes its target once
/// the driving temperature has moved at least this far from the temperature at
/// its last commanded change, so sensor jitter around a curve knee doesn't make
/// the fan hunt up and down every second.
#[cfg(windows)]
const CURVE_HYSTERESIS_C: f32 = 1.5;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct FanConfig {
    /// Legacy Phase 1 field from before tach channels and PWM channels were
    /// split. Migrated into `confirmed_rpm_channels` on load.
    #[serde(default)]
    pub confirmed_channel: Option<u8>,
    #[serde(default)]
    pub confirmed_rpm_channels: Vec<u8>,
    /// Discovered tach-channel → PWM-header mapping (Phase 2). Keyed by tach
    /// channel index, value is the PWM header index.
    #[serde(default)]
    pub pwm_map: std::collections::HashMap<u8, u8>,
    #[serde(default)]
    pub limits: std::collections::HashMap<u8, FanLimits>,
}

/// One point on a fan curve, pushed from the frontend as part of a control plan.
#[derive(Debug, Clone, Deserialize)]
pub struct CurvePoint {
    #[serde(rename = "tempC")]
    pub temp_c: f32,
    #[serde(rename = "speedPct")]
    pub speed_pct: f32,
}

/// How one fan should be driven: a fixed manual duty, or a temperature curve.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum FanControlMode {
    Manual {
        pct: f32,
    },
    Curve {
        #[serde(rename = "tempSource")]
        temp_source: String,
        points: Vec<CurvePoint>,
    },
}

/// The background fan-control plan, pushed by the UI whenever the user changes a
/// mode, edits a curve, or engages/releases control. The backend control loop
/// owns it from then on, so fans keep tracking temperature even when the window
/// is hidden and its JS timers are throttled.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanControlPlan {
    /// While true the loop asserts nothing (user pressed STOP → BIOS).
    pub released: bool,
    /// Mode for a mapped fan with no explicit per-channel entry (e.g. a fan
    /// freshly found by burst auto-detect). `None` leaves such fans on the BIOS.
    pub default_mode: Option<FanControlMode>,
    /// Per tach-channel mode. Channels not in the device's PWM map are ignored.
    pub modes: std::collections::HashMap<u8, FanControlMode>,
}

/// Non-Windows builds have no ring-0 path; these types still need to exist so
/// the Tauri commands compile cross-platform. They are never produced there.
#[cfg(not(windows))]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelReading {
    pub index: u8,
    pub label: String,
    pub rpm: u16,
    pub pwm_pct: Option<u8>,
    pub manual: Option<bool>,
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempReading {
    pub key: String,
    pub label: String,
    pub temp_c: f32,
}

#[cfg(not(windows))]
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepResult {
    pub max_rpm: u16,
    pub min_running_pct: u8,
    pub min_running_rpm: u16,
    pub stall_pct: Option<u8>,
    pub samples: Vec<(u8, u16)>,
}

#[derive(Debug, thiserror::Error)]
pub enum FanError {
    #[error("fan control unavailable: {0}")]
    Unavailable(String),
    #[cfg(windows)]
    #[error("chip error: {0}")]
    Chip(#[from] nct6687::ChipError),
    #[error("write refused: {0}")]
    Refused(String),
}

/// One mapped, controllable fan, surfaced to the frontend so it can draw a
/// slider with the right bounds.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanMapping {
    /// Tach channel the user confirmed.
    pub rpm_channel: u8,
    /// PWM header that drives it (discovered by the mapping probe).
    pub header: u8,
    pub header_label: String,
    /// Effective floor (%) a `set_speed` will clamp up to. May be below the
    /// conservative default once a stall has been measured.
    pub min_pwm: u8,
    /// Ceiling (%).
    pub max_pwm: u8,
    pub measured_stall_rpm: Option<u16>,
    pub measured_max_rpm: Option<u16>,
}

/// Outcome of a burst auto-detect run, surfaced to the UI for feedback.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct BurstResult {
    /// Tach channels found spinning after the burst and auto-mapped to their
    /// header. Excludes the pump, which is never controlled.
    pub detected: Vec<u8>,
}

/// Per-header live stats for one burst sample, streamed to the debug modal.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstFanProgress {
    pub header: u8,
    pub header_label: String,
    pub rpm_channel: u8,
    /// RPM read this sample.
    pub rpm: u16,
    /// Highest RPM seen so far this run.
    pub max_rpm: u16,
    /// A fan is spinning on this header right now (rpm > 0).
    pub detected: bool,
}

/// One burst sample snapshot, emitted as `fan-burst-progress` for the debug UI.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BurstProgress {
    /// Milliseconds since the burst began.
    pub elapsed_ms: u64,
    /// Total burst window (the configured hold), so the UI can show a countdown.
    pub total_ms: u64,
    /// "bursting" while the window runs, "done" for the final snapshot.
    pub phase: &'static str,
    pub fans: Vec<BurstFanProgress>,
}

/// Snapshot returned to the frontend.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanStatus {
    /// True if the driver loaded and a supported chip was found.
    pub available: bool,
    /// Human-readable reason when `available` is false (no admin, no driver,
    /// unsupported board).
    pub detail: String,
    /// Detected chip id (e.g. "NCT6687D-R (0xD592)"), when available.
    pub chip: Option<String>,
    /// RPM/tach channels the user has confirmed are case fans.
    pub confirmed_rpm_channels: Vec<u8>,
    /// Whether speed *writes* are possible (chip available). Phase 2: true once
    /// the chip is detected, even before any channel is mapped.
    pub writes_enabled: bool,
    /// True while KontrolRGB is holding at least one header in manual control.
    /// Arms STOP and the heartbeat.
    pub manual_active: bool,
    /// Mapped, controllable fans (tach channel → PWM header) with their bounds.
    pub mappings: Vec<FanMapping>,
}

/// Combined fan page payload, fetched in one backend call so the frontend does
/// not queue separate status/read/temp commands against the same chip lock.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FanSnapshot {
    pub status: FanStatus,
    pub readings: Vec<ChannelReading>,
    pub temps: Vec<TempReading>,
}

/// The subsystem. Cheap to construct; hardware is opened lazily on first use so
/// app startup never blocks on (or fails from) a missing driver.
pub struct FanSubsystem {
    inner: Arc<Mutex<Inner>>,
    /// Cancel flag for an in-flight `sweep`. Lives OUTSIDE the mutex so the Stop
    /// button can set it while the sweep thread holds the lock for the whole run.
    sweep_cancel: Arc<AtomicBool>,
    /// Suspends the control loop for the duration of a burst/sweep so it can't
    /// write the chip between the long op's lock-release windows.
    #[cfg_attr(not(windows), allow(dead_code))]
    control_suspend: Arc<AtomicBool>,
    /// Shared with `AppState`: the control loop refuses to write until competing
    /// RGB/fan software has been cleared.
    #[cfg_attr(not(windows), allow(dead_code))]
    conflicts_cleared: Arc<AtomicBool>,
    /// Shared with `AppState`: lets a fan curve be driven off the GPU die temp.
    /// Read each control pass and surfaced as the synthetic `"gpu"` temp source.
    #[cfg_attr(not(windows), allow(dead_code))]
    gpu_telemetry: Arc<GpuTelemetrySubsystem>,
}

struct Inner {
    #[cfg(windows)]
    chip: Option<Nct6687>,
    /// Last detection error, surfaced as `FanStatus.detail`.
    detail: String,
    /// RPM/tach channel indexes the user confirmed via the mapping wizard.
    confirmed_rpm_channels: Vec<u8>,
    /// Discovered tach channel → PWM header mapping.
    pwm_map: std::collections::HashMap<u8, u8>,
    /// Per-header safety limits (keyed by header index). Defaults applied lazily.
    limits: std::collections::HashMap<u8, FanLimits>,
    /// True while this process holds manual control of at least one header.
    /// Drives STOP and the watchdog; cleared by `release_to_bios`.
    manual_session_active: bool,
    /// Last time the UI pinged. The watchdog releases control if this goes stale.
    #[cfg(windows)]
    last_heartbeat: Instant,
    /// Set once the watchdog thread has been spawned (lazily, on first write).
    #[cfg(windows)]
    watchdog_started: bool,
    /// Baseline EC snapshot for the passive capture/diff tool (`ec_capture`).
    #[cfg(windows)]
    ec_baseline: Option<Vec<(u16, u8)>>,
    /// Background control plan pushed from the UI; owned by the control loop.
    #[cfg(windows)]
    control_plan: Option<FanControlPlan>,
    /// Set once the control loop thread has been spawned (lazily, on first plan).
    #[cfg(windows)]
    control_started: bool,
    /// Last duty (%) the control loop commanded per tach channel (2% dedup).
    #[cfg(windows)]
    control_last_commanded: std::collections::HashMap<u8, u8>,
    /// Driving temperature at each channel's last commanded change (curve
    /// hysteresis): a curve fan holds until its source moves `CURVE_HYSTERESIS_C`
    /// from this, so jitter around a knee can't make it hunt.
    #[cfg(windows)]
    control_last_temp: std::collections::HashMap<u8, f32>,
    /// True while the thermal failsafe is forcing fans to 100%. Kept so the loop
    /// applies the clear-hysteresis band and logs the engage/clear edges once.
    #[cfg(windows)]
    emergency_active: bool,
    /// Crash-recovery marker file. Written while we hold manual control so the
    /// next launch can detect an unclean exit that stranded a fan; removed on a
    /// clean handoff back to the BIOS. `None` until wired at startup.
    #[cfg(windows)]
    recovery_marker: Option<std::path::PathBuf>,
    /// Whether the marker file currently exists, so arming/disarming touches the
    /// disk only on the actual transition, not every control pass.
    #[cfg(windows)]
    marker_armed: bool,
}

impl FanSubsystem {
    pub fn new(
        conflicts_cleared: Arc<AtomicBool>,
        gpu_telemetry: Arc<GpuTelemetrySubsystem>,
    ) -> Self {
        FanSubsystem {
            inner: Arc::new(Mutex::new(Inner {
                #[cfg(windows)]
                chip: None,
                detail: "not initialized".to_string(),
                confirmed_rpm_channels: Vec::new(),
                pwm_map: Default::default(),
                limits: Default::default(),
                manual_session_active: false,
                #[cfg(windows)]
                last_heartbeat: Instant::now(),
                #[cfg(windows)]
                watchdog_started: false,
                #[cfg(windows)]
                ec_baseline: None,
                #[cfg(windows)]
                control_plan: None,
                #[cfg(windows)]
                control_started: false,
                #[cfg(windows)]
                control_last_commanded: Default::default(),
                #[cfg(windows)]
                control_last_temp: Default::default(),
                #[cfg(windows)]
                emergency_active: false,
                #[cfg(windows)]
                recovery_marker: None,
                #[cfg(windows)]
                marker_armed: false,
            })),
            sweep_cancel: Arc::new(AtomicBool::new(false)),
            control_suspend: Arc::new(AtomicBool::new(false)),
            conflicts_cleared,
            gpu_telemetry,
        }
    }

    /// Request cancellation of an in-flight sweep. Lock-free so it works while the
    /// sweep thread holds the subsystem mutex; the sweep loop releases the header
    /// back to the BIOS and bails on the next slice. No-op if nothing is sweeping.
    pub fn cancel_sweep(&self) {
        self.sweep_cancel.store(true, Ordering::Relaxed);
    }

    /// Wire the crash-recovery marker path (once, at startup) and reclaim any fan
    /// a previous unclean exit may have stranded. If the marker file already
    /// exists, the last session died holding manual control — an in-process
    /// watchdog can't fire after a SIGKILL/power event — so force every persisted
    /// mapped header back to the BIOS curve before the user touches anything.
    /// Idempotent: no marker → nothing to do; chip unavailable → the marker is
    /// kept so a later (elevated/driver-present) launch retries. Must run AFTER
    /// the persisted config (the `pwm_map`) has been applied.
    #[cfg(windows)]
    pub fn init_recovery(&self, marker: std::path::PathBuf) {
        let stranded = marker.exists();
        let mut inner = self.inner.lock();
        inner.recovery_marker = Some(marker);
        if !stranded {
            return;
        }
        // Reflect the on-disk file so a successful reclaim removes it.
        inner.marker_armed = true;
        Self::ensure_open(&mut inner);
        let headers: Vec<u8> = inner
            .pwm_map
            .values()
            .copied()
            .filter(|&h| h != PUMP_HEADER_INDEX)
            .collect();
        if let Some(chip) = inner.chip.as_ref() {
            for h in headers {
                let _ = chip.force_release_header(h as usize);
            }
            eprintln!(
                "[fan recovery] unclean exit detected — {} mapped header(s) forced back to BIOS",
                inner.pwm_map.len()
            );
            disarm_marker(&mut inner);
        } else {
            eprintln!("[fan recovery] marker present but chip unavailable — will retry next launch");
        }
    }

    #[cfg(not(windows))]
    pub fn init_recovery(&self, _marker: std::path::PathBuf) {}

    /// Ensure the chip is open. Non-fatal: a `None` chip just means "unavailable".
    #[cfg(windows)]
    fn ensure_open(inner: &mut Inner) {
        if inner.chip.is_some() {
            return;
        }
        match lpc::Lpc::open() {
            Ok(lpc) => match Nct6687::detect(lpc) {
                Ok(chip) => {
                    inner.detail = "ok".to_string();
                    inner.chip = Some(chip);
                }
                Err(e) => inner.detail = e.to_string(),
            },
            Err(e) => inner.detail = e.to_string(),
        }
    }

    #[cfg(windows)]
    fn mappings_snapshot(inner: &Inner) -> Vec<FanMapping> {
        let mut maps: Vec<FanMapping> = inner
            .pwm_map
            .iter()
            .map(|(&rpm_channel, &header)| {
                let limits = inner.limits.get(&header).cloned().unwrap_or_default();
                FanMapping {
                    rpm_channel,
                    header,
                    header_label: nct6687::HEADER_LABELS
                        .get(header as usize)
                        .copied()
                        .unwrap_or("?")
                        .to_string(),
                    min_pwm: limits.effective_floor(),
                    max_pwm: limits.max_pwm.min(100),
                    measured_stall_rpm: limits.measured_stall_rpm,
                    measured_max_rpm: limits.measured_max_rpm,
                }
            })
            .collect();
        maps.sort_by_key(|m| m.rpm_channel);
        maps
    }

    #[cfg(windows)]
    fn status_from_inner(inner: &Inner) -> FanStatus {
        let chip = inner
            .chip
            .as_ref()
            .map(|c| format!("NCT6687D-R (0x{:04X})", c.chip_id));
        let available = inner.chip.is_some();
        FanStatus {
            available,
            detail: inner.detail.clone(),
            chip,
            confirmed_rpm_channels: inner.confirmed_rpm_channels.clone(),
            writes_enabled: available,
            manual_active: inner.manual_session_active,
            mappings: Self::mappings_snapshot(inner),
        }
    }

    fn reject_pump_header(header: u8) -> Result<(), FanError> {
        if header == PUMP_HEADER_INDEX {
            Err(FanError::Refused(
                "pump header is read-only and cannot be controlled".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    #[cfg(windows)]
    pub fn status(&self) -> FanStatus {
        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        Self::status_from_inner(&inner)
    }

    #[cfg(not(windows))]
    pub fn status(&self) -> FanStatus {
        let inner = self.inner.lock();
        FanStatus {
            available: false,
            detail: "fan control is Windows-only".to_string(),
            chip: None,
            confirmed_rpm_channels: inner.confirmed_rpm_channels.clone(),
            writes_enabled: false,
            manual_active: false,
            mappings: Vec::new(),
        }
    }

    #[cfg(windows)]
    pub fn snapshot(&self) -> Result<FanSnapshot, FanError> {
        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        let status = Self::status_from_inner(&inner);
        let Some(chip) = inner.chip.as_ref() else {
            return Ok(FanSnapshot {
                status,
                readings: Vec::new(),
                temps: Vec::new(),
            });
        };
        let readings = chip.read_all()?;
        let temps = chip.read_temps()?;
        Ok(FanSnapshot {
            status,
            readings,
            temps,
        })
    }

    #[cfg(not(windows))]
    pub fn snapshot(&self) -> Result<FanSnapshot, FanError> {
        Ok(FanSnapshot {
            status: self.status(),
            readings: Vec::new(),
            temps: Vec::new(),
        })
    }

    /// Live readings for every channel. Pure read.
    #[cfg(windows)]
    pub fn read(&self) -> Result<Vec<ChannelReading>, FanError> {
        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        Ok(chip.read_all()?)
    }

    #[cfg(not(windows))]
    pub fn read(&self) -> Result<Vec<ChannelReading>, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Read temperature sensors from the chip. Returns only sensors with
    /// plausible readings. Fast — pure EC reads, no fan interaction.
    #[cfg(windows)]
    pub fn read_temps(&self) -> Result<Vec<TempReading>, FanError> {
        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        Ok(chip.read_temps()?)
    }

    #[cfg(not(windows))]
    pub fn read_temps(&self) -> Result<Vec<TempReading>, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Passive EC capture: snapshot the fan/HWM register space and diff it
    /// against a stored baseline. Call with label `"baseline"` to store the
    /// reference; call again (any other label) AFTER an external tool (MSI
    /// Center) has changed a SYS fan to get back the list of `(addr, old, new)`
    /// bytes that changed — i.e. the registers that tool actually drives. Pure
    /// reads; never writes the chip.
    #[cfg(windows)]
    pub fn ec_capture(&self, label: String) -> Result<Vec<(u16, u8, u8)>, FanError> {
        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        let snap = {
            let chip = inner
                .chip
                .as_ref()
                .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
            chip.scan_ec()
        };
        if label == "baseline" || inner.ec_baseline.is_none() {
            inner.ec_baseline = Some(snap);
            eprintln!(
                "[ec capture] baseline stored ({} regs)",
                inner.ec_baseline.as_ref().unwrap().len()
            );
            return Ok(Vec::new());
        }
        let base = inner.ec_baseline.clone().unwrap();
        let mut diffs = Vec::new();
        for ((addr, new), (_, old)) in snap.iter().zip(base.iter()) {
            if new != old {
                diffs.push((*addr, *old, *new));
                eprintln!("[ec diff:{label}] 0x{addr:03X}: 0x{old:02X} -> 0x{new:02X}");
            }
        }
        eprintln!("[ec capture] '{label}': {} changed regs", diffs.len());
        Ok(diffs)
    }

    #[cfg(not(windows))]
    pub fn ec_capture(&self, _label: String) -> Result<Vec<(u16, u8, u8)>, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// STOP / panic: hand every header back to the BIOS fan curve.
    #[cfg(windows)]
    pub fn release_to_bios(&self) -> Result<(), FanError> {
        let mut inner = self.inner.lock();
        // Mark the plan released backend-side too, so the control loop can't
        // re-assert a fan in the window between this release and the frontend's
        // own (fire-and-forget) released-plan push landing.
        if let Some(plan) = inner.control_plan.as_mut() {
            plan.released = true;
        }
        if !inner.manual_session_active {
            return Ok(());
        }
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        chip.release_to_bios()?;
        inner.manual_session_active = false;
        disarm_marker(&mut inner);
        clear_control_cache(&mut inner);
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn release_to_bios(&self) -> Result<(), FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Record the RPM/tach channel the user confirmed (via watching live RPM)
    /// is one of their case fans. Re-reads the chip and refuses a channel that
    /// is not currently reporting a live RPM signal. Does not prove which PWM
    /// header owns the fan — that is `map_pwm_header`.
    #[cfg(windows)]
    pub fn confirm_channel(&self, index: u8) -> Result<(), FanError> {
        if index >= RPM_CHANNEL_COUNT {
            return Err(FanError::Refused(format!(
                "channel {index} is not a reported RPM channel"
            )));
        }

        let mut inner = self.inner.lock();
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        let readings = chip.read_all()?;
        let reading = readings
            .iter()
            .find(|r| r.index == index)
            .ok_or_else(|| FanError::Refused(format!("channel {index} was not reported")))?;
        if reading.rpm == 0 {
            return Err(FanError::Refused(format!(
                "channel {index} has no live RPM signal"
            )));
        }

        if !inner.confirmed_rpm_channels.contains(&index) {
            inner.confirmed_rpm_channels.push(index);
            inner.confirmed_rpm_channels.sort_unstable();
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn confirm_channel(&self, _index: u8) -> Result<(), FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Map a confirmed tach channel to the PWM header that drives it, using the
    /// verified msi_alt register map (a pure lookup — no spinning, no hardware
    /// access). e.g. ch11 → header 6, ch12 → header 5. The user verifies the
    /// pairing afterwards by nudging the speed and watching the live RPM.
    #[cfg(windows)]
    pub fn map_pwm_header(&self, rpm_channel: u8) -> Result<u8, FanError> {
        let mut inner = self.inner.lock();
        if !inner.confirmed_rpm_channels.contains(&rpm_channel) {
            return Err(FanError::Refused(format!(
                "channel {rpm_channel} is not a confirmed case-fan channel"
            )));
        }
        let header = nct6687::header_for_rpm_channel(rpm_channel as usize).ok_or_else(|| {
            FanError::Refused(format!(
                "tach channel {rpm_channel} has no controllable PWM header on this board"
            ))
        })?;
        let header = header as u8;
        Self::reject_pump_header(header)?;
        inner.pwm_map.insert(rpm_channel, header);
        inner.limits.entry(header).or_default();
        Ok(header)
    }

    #[cfg(not(windows))]
    pub fn map_pwm_header(&self, _rpm_channel: u8) -> Result<u8, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Burst auto-detect: drive every controllable header (except the pump) to
    /// 100%, hold for a fixed `duration_secs` window while sampling RPM (so the
    /// debug modal shows the live spin-up), then map every header still spinning
    /// at the end as a controllable fan. No acceleration/plateau detection — a
    /// fixed hold is simpler and reliably catches slow movers (SYS_FAN 5/6) as
    /// long as the window is long enough. The pump (`PUMP_HEADER_INDEX`) is never
    /// bursted or mapped, and every header is released back to the BIOS before
    /// returning so nothing is stranded at full duty.
    #[cfg(windows)]
    pub fn burst_detect(
        &self,
        duration_secs: u64,
        on_progress: impl Fn(BurstProgress),
    ) -> Result<BurstResult, FanError> {
        use nct6687::rpm_channel_for_header;
        use std::collections::HashMap;

        // Hold the control loop off the chip for the whole burst (auto-restored).
        let _suspend = SuspendGuard::new(self.control_suspend.clone());

        /// How often we sample RPM during the burst.
        const SAMPLE: Duration = Duration::from_millis(500);

        // Clamp the configured hold to a sane band (default is set frontend-side).
        let hold = Duration::from_secs(duration_secs.clamp(2, 30));
        let total_ms = hold.as_millis() as u64;

        let label = |h: u8| {
            nct6687::HEADER_LABELS
                .get(h as usize)
                .copied()
                .unwrap_or("?")
                .to_string()
        };

        // Non-pump headers that have a tach channel, paired (header, tach).
        let mut header_tach: Vec<(u8, usize)> = Vec::new();
        for header in 0..CONTROLLABLE_CHANNEL_COUNT {
            if header == PUMP_HEADER_INDEX {
                continue;
            }
            if let Some(ch) = rpm_channel_for_header(header as usize) {
                header_tach.push((header, ch));
            }
        }

        // Phase 1 — burst every non-pump header to full duty. The session flag,
        // heartbeat, and crash marker are armed BEFORE the first write: if a
        // write fails partway, the headers already at 100% must still be covered
        // by the watchdog and crash recovery, not silently stranded.
        {
            let mut inner = self.inner.lock();
            Self::ensure_open(&mut inner);
            if inner.chip.is_none() {
                return Err(FanError::Unavailable(inner.detail.clone()));
            }
            inner.manual_session_active = true;
            inner.last_heartbeat = Instant::now();
            arm_marker(&mut inner);
            let chip = inner.chip.as_ref().expect("checked above");
            let mut write_err: Option<nct6687::ChipError> = None;
            for &(header, _) in &header_tach {
                if let Err(e) = chip.set_manual_pwm(header as usize, 255) {
                    write_err = Some(e);
                    break;
                }
            }
            if let Some(e) = write_err {
                // Hand back whatever we already took, then undo the session arm.
                let _ = chip.release_to_bios();
                inner.manual_session_active = false;
                disarm_marker(&mut inner);
                clear_control_cache(&mut inner);
                return Err(e.into());
            }
        }

        // Phase 2 — hold the burst for the fixed window, sampling RPM so the modal
        // can show live spin-up. The lock is released during each sleep so the UI
        // can still read live RPM.
        let start = Instant::now();
        let mut max_seen: HashMap<u8, u16> = HashMap::new();
        let mut last_rpm: HashMap<u8, u16> = HashMap::new();
        let mut read_err: Option<FanError> = None;

        while start.elapsed() < hold {
            std::thread::sleep(SAMPLE);

            let mut inner = self.inner.lock();
            // Keep the watchdog from reclaiming the fans mid-burst.
            inner.last_heartbeat = Instant::now();
            let readings = match inner
                .chip
                .as_ref()
                .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))
                .and_then(|c| c.read_all().map_err(FanError::from))
            {
                Ok(r) => r,
                Err(e) => {
                    read_err = Some(e);
                    break;
                }
            };
            let mut rpm_by_ch: HashMap<usize, u16> = HashMap::new();
            for r in &readings {
                rpm_by_ch.insert(r.index as usize, r.rpm);
            }
            drop(inner);

            let rows: Vec<BurstFanProgress> = header_tach
                .iter()
                .map(|&(header, ch)| {
                    let rpm = *rpm_by_ch.get(&ch).unwrap_or(&0);
                    let m = max_seen.entry(header).or_insert(0);
                    *m = (*m).max(rpm);
                    last_rpm.insert(header, rpm);
                    BurstFanProgress {
                        header,
                        header_label: label(header),
                        rpm_channel: ch as u8,
                        rpm,
                        max_rpm: *m,
                        detected: rpm > 0,
                    }
                })
                .collect();

            on_progress(BurstProgress {
                elapsed_ms: start.elapsed().as_millis() as u64,
                total_ms,
                phase: "bursting",
                fans: rows,
            });
        }

        // Phase 3 — always release to BIOS, then map every header still spinning.
        let mut inner = self.inner.lock();
        if let Some(chip) = inner.chip.as_ref() {
            let _ = chip.release_to_bios();
        }
        inner.manual_session_active = false;
        disarm_marker(&mut inner);
        // Headers just went back to the BIOS; the control loop's write-dedup
        // cache is now stale (it thinks its last duty is still applied), so
        // clear it or the loop would skip re-asserting an unchanged target.
        clear_control_cache(&mut inner);
        if let Some(e) = read_err {
            return Err(e);
        }

        let mut detected = Vec::new();
        for &(header, ch) in &header_tach {
            if *last_rpm.get(&header).unwrap_or(&0) == 0 {
                continue;
            }
            let ch = ch as u8;
            if !inner.confirmed_rpm_channels.contains(&ch) {
                inner.confirmed_rpm_channels.push(ch);
            }
            inner.pwm_map.insert(ch, header);
            inner.limits.entry(header).or_default();
            detected.push(ch);
        }
        inner.confirmed_rpm_channels.sort_unstable();
        inner.confirmed_rpm_channels.dedup();
        detected.sort_unstable();

        // Final snapshot so the debug modal can freeze on the end state.
        let done_rows: Vec<BurstFanProgress> = header_tach
            .iter()
            .map(|&(header, ch)| {
                let rpm = *last_rpm.get(&header).unwrap_or(&0);
                BurstFanProgress {
                    header,
                    header_label: label(header),
                    rpm_channel: ch as u8,
                    rpm,
                    max_rpm: *max_seen.get(&header).unwrap_or(&0),
                    detected: rpm > 0,
                }
            })
            .collect();
        on_progress(BurstProgress {
            elapsed_ms: start.elapsed().as_millis() as u64,
            total_ms,
            phase: "done",
            fans: done_rows,
        });

        Ok(BurstResult { detected })
    }

    #[cfg(not(windows))]
    pub fn burst_detect(
        &self,
        _duration_secs: u64,
        _on_progress: impl Fn(BurstProgress),
    ) -> Result<BurstResult, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Set a mapped fan's speed (% duty), clamped to its safe window. Takes
    /// manual control of the header and arms the watchdog. Returns the clamped
    /// percentage actually applied.
    #[cfg(windows)]
    pub fn set_speed(&self, rpm_channel: u8, pct: u8) -> Result<u8, FanError> {
        let need_watchdog;
        let applied;
        {
            let mut inner = self.inner.lock();
            let &header = inner.pwm_map.get(&rpm_channel).ok_or_else(|| {
                FanError::Refused(format!("channel {rpm_channel} has no mapped PWM header"))
            })?;
            Self::reject_pump_header(header)?;
            let limits = inner.limits.get(&header).cloned().unwrap_or_default();
            let clamped = limits.clamp_pct(pct);
            let duty = ((clamped as u16 * 255) / 100) as u8;

            Self::ensure_open(&mut inner);
            let chip = inner
                .chip
                .as_ref()
                .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
            chip.set_manual_pwm(header as usize, duty)?;

            inner.manual_session_active = true;
            inner.last_heartbeat = Instant::now();
            arm_marker(&mut inner);
            need_watchdog = !inner.watchdog_started;
            if need_watchdog {
                inner.watchdog_started = true;
            }
            applied = clamped;
        }
        if need_watchdog {
            start_watchdog(self.inner.clone());
        }
        Ok(applied)
    }

    #[cfg(not(windows))]
    pub fn set_speed(&self, _rpm_channel: u8, _pct: u8) -> Result<u8, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Sweep a mapped fan's duty down to find its real stall point and top RPM,
    /// then fold the result into that header's safety limits. Spins the fan
    /// through its range (~30s) and restores it to the BIOS when done. This is
    /// the only path allowed below the conservative floor.
    #[cfg(windows)]
    pub fn sweep(
        &self,
        rpm_channel: u8,
        on_progress: impl Fn(u8, u16, &'static str),
    ) -> Result<SweepResult, FanError> {
        // Hold the control loop off the chip for the whole sweep (auto-restored).
        let _suspend = SuspendGuard::new(self.control_suspend.clone());
        // Clear any stale cancel request from a previous run before we begin.
        self.sweep_cancel.store(false, Ordering::Relaxed);
        let mut inner = self.inner.lock();
        let &header = inner.pwm_map.get(&rpm_channel).ok_or_else(|| {
            FanError::Refused(format!("channel {rpm_channel} has no mapped PWM header"))
        })?;
        Self::reject_pump_header(header)?;
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        let result = chip.sweep_header(
            header as usize,
            rpm_channel as usize,
            &self.sweep_cancel,
            &on_progress,
        );
        if result.is_err() {
            // Cancel releases the header itself, but a mid-sweep chip error can
            // leave it held at the last duty. release_to_bios is a no-op for
            // headers already handed back, so this is safe on every error path.
            let _ = chip.release_to_bios();
        }
        // On EVERY exit — success, cancel, or error — the header is back on the
        // BIOS, so the session flag and the control loop's write-dedup cache
        // must reset; a stale cache would make the loop skip re-asserting an
        // unchanged target and silently leave the fan on the BIOS curve.
        inner.manual_session_active = false;
        clear_control_cache(&mut inner);
        let result = result?;
        inner.limits.entry(header).or_default().apply_sweep(&result);
        Ok(result)
    }

    #[cfg(not(windows))]
    pub fn sweep(
        &self,
        _rpm_channel: u8,
        _on_progress: impl Fn(u8, u16, &'static str),
    ) -> Result<SweepResult, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// Sweep every mapped, non-pump fan simultaneously ("Calibrate all"): all
    /// fans walk the same duty ladder together, so a whole-system calibration
    /// takes roughly one sweep's wall time instead of one per fan. Folds each
    /// result into that header's safety limits, exactly like a single `sweep`.
    /// `on_progress(rpm_channel, pct, rpm, phase)` streams per-fan samples
    /// (phase `"settling"` / `"measuring"` / `"done"`). Returns
    /// `(rpm_channel, result)` pairs.
    #[cfg(windows)]
    pub fn sweep_all(
        &self,
        on_progress: impl Fn(u8, u8, u16, &'static str),
    ) -> Result<Vec<(u8, SweepResult)>, FanError> {
        // Hold the control loop off the chip for the whole sweep (auto-restored).
        let _suspend = SuspendGuard::new(self.control_suspend.clone());
        // Clear any stale cancel request from a previous run before we begin.
        self.sweep_cancel.store(false, Ordering::Relaxed);
        let mut inner = self.inner.lock();
        let mut targets: Vec<(usize, usize)> = inner
            .pwm_map
            .iter()
            .filter(|&(_, &header)| header != PUMP_HEADER_INDEX)
            .map(|(&rpm_channel, &header)| (header as usize, rpm_channel as usize))
            .collect();
        targets.sort_by_key(|&(_, ch)| ch);
        if targets.is_empty() {
            return Err(FanError::Refused("no mapped fans to calibrate".to_string()));
        }
        Self::ensure_open(&mut inner);
        let chip = inner
            .chip
            .as_ref()
            .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
        let results = chip.sweep_headers(&targets, &self.sweep_cancel, &|channel, pct, rpm, phase| {
            on_progress(channel as u8, pct, rpm, phase)
        });
        if results.is_err() {
            // Cancel releases the headers itself, but a mid-sweep chip error can
            // leave some held; release_to_bios is a no-op for headers already
            // handed back, so this is safe on every error path.
            let _ = chip.release_to_bios();
        }
        // Same cleanup contract as `sweep`: on every exit the headers are back
        // on the BIOS, so reset the session flag and the control loop's
        // write-dedup cache (a stale cache would strand fans on the BIOS).
        inner.manual_session_active = false;
        clear_control_cache(&mut inner);
        let results = results?;
        let mut out = Vec::with_capacity(targets.len());
        for (&(header, rpm_channel), result) in targets.iter().zip(results) {
            inner
                .limits
                .entry(header as u8)
                .or_default()
                .apply_sweep(&result);
            out.push((rpm_channel as u8, result));
        }
        Ok(out)
    }

    #[cfg(not(windows))]
    pub fn sweep_all(
        &self,
        _on_progress: impl Fn(u8, u8, u16, &'static str),
    ) -> Result<Vec<(u8, SweepResult)>, FanError> {
        Err(FanError::Unavailable(
            "fan control is Windows-only".to_string(),
        ))
    }

    /// UI heartbeat for the manual `set_speed` path. When a fan is held by a
    /// direct slider write (no control *plan* pushed), the background control
    /// loop isn't the heartbeat source — this poll is — so it is NOT vestigial:
    /// it's what keeps the watchdog from reclaiming a slider-held fan between
    /// writes. Curve/plan sessions beat from `control_pass` instead. Uses
    /// `try_lock` so it never blocks behind a long hardware op (a missed beat or
    /// two is fine — the watchdog timeout is several poll intervals wide).
    #[cfg(windows)]
    pub fn heartbeat(&self) {
        if let Some(mut inner) = self.inner.try_lock() {
            inner.last_heartbeat = Instant::now();
        }
    }

    #[cfg(not(windows))]
    pub fn heartbeat(&self) {}

    /// Install/replace the background control plan and ensure the control loop is
    /// running. The loop then drives every mapped fan to its target each
    /// `CONTROL_INTERVAL`, independent of the UI's (throttleable) timers. Pushing
    /// a new plan re-evaluates every fan on the next pass.
    #[allow(unused_variables)]
    pub fn set_control_plan(&self, plan: FanControlPlan) {
        #[cfg(windows)]
        {
            let need_start;
            let need_watchdog;
            {
                let mut inner = self.inner.lock();
                // Pre-sort every curve's points once, here, so the per-second
                // control pass can interpolate without re-sorting/allocating.
                let mut plan = plan;
                for mode in plan.modes.values_mut() {
                    if let FanControlMode::Curve { points, .. } = mode {
                        points.sort_by(|a, b| {
                            a.temp_c
                                .partial_cmp(&b.temp_c)
                                .unwrap_or(std::cmp::Ordering::Equal)
                        });
                    }
                }
                if let Some(FanControlMode::Curve { points, .. }) = plan.default_mode.as_mut() {
                    points.sort_by(|a, b| {
                        a.temp_c
                            .partial_cmp(&b.temp_c)
                            .unwrap_or(std::cmp::Ordering::Equal)
                    });
                }
                inner.control_plan = Some(plan);
                // Force a re-apply on the next pass; the config just changed.
                inner.control_last_commanded.clear();
                inner.control_last_temp.clear();
                need_start = !inner.control_started;
                if need_start {
                    inner.control_started = true;
                }
                // The control loop is now the fans' sole heartbeat source, so its
                // failsafe MUST be armed here — not only on the manual `set_speed`
                // path. A curve-only session that never touched a slider would
                // otherwise run with no watchdog: if the control thread died while
                // holding a fan low, nothing would hand it back to the BIOS.
                need_watchdog = !inner.watchdog_started;
                if need_watchdog {
                    inner.watchdog_started = true;
                }
            }
            if need_start {
                start_control_loop(
                    self.inner.clone(),
                    self.control_suspend.clone(),
                    self.conflicts_cleared.clone(),
                    self.gpu_telemetry.clone(),
                );
            }
            if need_watchdog {
                start_watchdog(self.inner.clone());
            }
        }
    }

    /// One control-loop pass: drive every mapped fan to its planned target.
    /// Curve fans interpolate against the live chip temps; manual fans hold their
    /// fixed duty. Skipped writes (≤2% from last) keep EC traffic down. Refreshes
    /// the watchdog heartbeat so a running loop is never reclaimed by the BIOS.
    #[cfg(windows)]
    fn control_pass(inner_arc: &Arc<Mutex<Inner>>, gpu: &Arc<GpuTelemetrySubsystem>) {
        let mut inner = inner_arc.lock();
        // Bail if released or no plan — without cloning the plan every pass.
        match inner.control_plan.as_ref() {
            None => return,
            Some(p) if p.released => return,
            Some(_) => {}
        }
        Self::ensure_open(&mut inner);

        // Die temp only — one NVML getter, not the full 5-query snapshot — so a
        // "gpu"-sourced curve tracks the card at minimal per-pass cost.
        // Unavailable → no "gpu" reading, and such a curve simply holds.
        let gpu_temp = gpu.read_temp_only();

        // Split disjoint field borrows so neither the plan nor the last-commanded
        // map is cloned each pass.
        let Inner {
            chip,
            pwm_map,
            limits,
            control_plan,
            control_last_commanded,
            control_last_temp,
            emergency_active,
            recovery_marker,
            marker_armed,
            manual_session_active,
            last_heartbeat,
            ..
        } = &mut *inner;

        let Some(plan) = control_plan.as_ref() else {
            return;
        };
        let Some(chip) = chip.as_ref() else {
            return;
        };
        let mut temps = match chip.read_temps() {
            Ok(t) => t,
            Err(_) => return,
        };
        if let Some(t) = gpu_temp {
            temps.push(TempReading {
                key: "gpu".to_string(),
                label: "GPU".to_string(),
                temp_c: t as f32,
            });
        }

        // Thermal failsafe. Trip when the hottest source reaches
        // EMERGENCY_TEMP_C; hold until everything drops below EMERGENCY_CLEAR_C
        // so a reading sitting on the threshold can't flap the fans. While
        // engaged, every controlled fan is forced to 100%, overriding both the
        // user's curve/manual duty and any max_pwm noise cap.
        let hottest = temps.iter().map(|r| r.temp_c).fold(f32::MIN, f32::max);
        let emergency = if *emergency_active {
            hottest >= EMERGENCY_CLEAR_C
        } else {
            hottest >= EMERGENCY_TEMP_C
        };
        if emergency != *emergency_active {
            if emergency {
                eprintln!(
                    "[fan] THERMAL FAILSAFE engaged at {hottest:.1}°C — forcing controlled fans to 100%"
                );
            } else {
                eprintln!(
                    "[fan] thermal failsafe cleared ({hottest:.1}°C) — resuming curve/manual control"
                );
                // The hysteresis anchors predate the emergency; left in place
                // they can hold curve fans at 100% until the source drifts
                // CURVE_HYSTERESIS_C from a stale reading. Re-anchor now.
                control_last_temp.clear();
            }
            *emergency_active = emergency;
        }

        let mut held = false;
        for (&rpm_channel, &header) in pwm_map.iter() {
            if header == PUMP_HEADER_INDEX {
                continue;
            }
            let Some(mode) = plan.modes.get(&rpm_channel).or(plan.default_mode.as_ref()) else {
                // Mapped but no mode/default → left on the BIOS. Do NOT flag the
                // session as manually held for a fan we never command.
                continue;
            };
            held = true;

            let clamped = if emergency {
                100
            } else {
                let raw = match mode {
                    FanControlMode::Manual { pct } => *pct,
                    FanControlMode::Curve {
                        temp_source,
                        points,
                    } => {
                        let Some(tr) = temps.iter().find(|r| r.key == *temp_source) else {
                            continue;
                        };
                        // Hysteresis: hold the current target unless the source
                        // has moved past the band since this fan last changed.
                        if control_last_commanded.contains_key(&rpm_channel) {
                            if let Some(&last_t) = control_last_temp.get(&rpm_channel) {
                                if (tr.temp_c - last_t).abs() < CURVE_HYSTERESIS_C {
                                    continue;
                                }
                            }
                        }
                        interpolate_curve(points, tr.temp_c)
                    }
                };
                let limits_h = limits.get(&header).cloned().unwrap_or_default();
                limits_h.clamp_pct(raw.round().clamp(0.0, 100.0) as u8)
            };

            let changed = control_last_commanded
                .get(&rpm_channel)
                .is_none_or(|&l| (clamped as i32 - l as i32).abs() > 2);
            if changed {
                let duty = ((clamped as u16 * 255) / 100) as u8;
                if chip.set_manual_pwm(header as usize, duty).is_ok() {
                    control_last_commanded.insert(rpm_channel, clamped);
                    // Record the driving temp at this change for the next
                    // hysteresis comparison (curve fans only, non-emergency).
                    if !emergency {
                        if let FanControlMode::Curve { temp_source, .. } = mode {
                            if let Some(tr) = temps.iter().find(|r| r.key == *temp_source) {
                                control_last_temp.insert(rpm_channel, tr.temp_c);
                            }
                        }
                    }
                }
            }
        }

        if held {
            *manual_session_active = true;
            *last_heartbeat = Instant::now();
            // Arm the crash-recovery marker (once). Inlined rather than calling
            // arm_marker() because we hold disjoint field borrows here, not a
            // whole &mut Inner.
            if !*marker_armed {
                if let Some(path) = recovery_marker.as_ref() {
                    if let Some(dir) = path.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    let _ = std::fs::write(path, b"kontrolrgb held manual fan control\n");
                }
                *marker_armed = true;
            }
        }
    }

    pub fn config(&self) -> FanConfig {
        let inner = self.inner.lock();
        FanConfig {
            confirmed_channel: None,
            confirmed_rpm_channels: inner.confirmed_rpm_channels.clone(),
            pwm_map: inner.pwm_map.clone(),
            limits: inner.limits.clone(),
        }
    }

    pub fn apply_config(&self, config: FanConfig) {
        let mut inner = self.inner.lock();
        let mut confirmed = config.confirmed_rpm_channels;
        if let Some(index) = config.confirmed_channel {
            confirmed.push(index);
        }
        confirmed.sort_unstable();
        confirmed.dedup();
        inner.confirmed_rpm_channels = confirmed
            .into_iter()
            .filter(|index| *index < RPM_CHANNEL_COUNT)
            .collect();
        inner.limits = config
            .limits
            .into_iter()
            .filter(|(index, _)| *index < CONTROLLABLE_CHANNEL_COUNT)
            .collect();
        inner.pwm_map = config
            .pwm_map
            .into_iter()
            .filter(|(rpm, header)| {
                *rpm < RPM_CHANNEL_COUNT
                    && *header < CONTROLLABLE_CHANNEL_COUNT
                    && *header != PUMP_HEADER_INDEX
            })
            .collect();
    }
}

/// Linear interpolation of a fan curve at `temp_c`, clamped to the endpoints.
/// Mirrors the frontend's `interpolateCurve` so previews match what we command.
/// `points` MUST be pre-sorted by `temp_c` ascending (done once in
/// `set_control_plan`), so this hot path allocates nothing and never re-sorts.
#[cfg(windows)]
fn interpolate_curve(points: &[CurvePoint], temp_c: f32) -> f32 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return 50.0;
    };
    if temp_c <= first.temp_c {
        return first.speed_pct;
    }
    if temp_c >= last.temp_c {
        return last.speed_pct;
    }
    for w in points.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if temp_c >= a.temp_c && temp_c <= b.temp_c {
            let span = b.temp_c - a.temp_c;
            if span <= 0.0 {
                return a.speed_pct;
            }
            let t = (temp_c - a.temp_c) / span;
            return a.speed_pct + t * (b.speed_pct - a.speed_pct);
        }
    }
    last.speed_pct
}

/// Background fan-control loop: every `CONTROL_INTERVAL`, drive each mapped fan
/// to its planned target. Runs on a dedicated OS thread, so it keeps tracking
/// temperature even when the window is hidden and its JS timers are throttled —
/// the reason the heartbeat watchdog can stay tight. Idles while the plan is
/// released, while a burst/sweep holds the chip (`suspend`), or before the user
/// has cleared conflicting software (`conflicts_cleared`).
#[cfg(windows)]
fn start_control_loop(
    inner: Arc<Mutex<Inner>>,
    suspend: Arc<AtomicBool>,
    conflicts_cleared: Arc<AtomicBool>,
    gpu_telemetry: Arc<GpuTelemetrySubsystem>,
) {
    std::thread::Builder::new()
        .name("fan-control".to_string())
        .spawn(move || loop {
            std::thread::sleep(CONTROL_INTERVAL);
            if suspend.load(Ordering::Relaxed) || !conflicts_cleared.load(Ordering::Relaxed) {
                continue;
            }
            FanSubsystem::control_pass(&inner, &gpu_telemetry);
        })
        .ok();
}

/// Suspends the control loop for its lifetime, restoring on drop. Used to fence
/// a burst/sweep so the loop can't write the chip between the op's lock-release
/// windows.
#[cfg(windows)]
struct SuspendGuard(Arc<AtomicBool>);

#[cfg(windows)]
impl SuspendGuard {
    fn new(flag: Arc<AtomicBool>) -> Self {
        flag.store(true, Ordering::Relaxed);
        SuspendGuard(flag)
    }
}

#[cfg(windows)]
impl Drop for SuspendGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

/// Write the crash-recovery marker (once) the first time we take manual control
/// in a session. Its presence at the next launch signals that this process may
/// have died holding a fan, so the fresh instance force-releases the persisted
/// mapped headers before doing anything else. Best-effort: a failed write only
/// costs us the recovery hint, never correctness.
#[cfg(windows)]
fn arm_marker(inner: &mut Inner) {
    if inner.marker_armed {
        return;
    }
    if let Some(path) = &inner.recovery_marker {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, b"kontrolrgb held manual fan control\n");
    }
    inner.marker_armed = true;
}

/// Reset the control loop's per-channel memory (write-dedup + curve-hysteresis
/// anchors). MUST be called whenever headers are handed back to the BIOS while
/// a plan may still be installed (STOP, watchdog, sweep/burst cleanup): the
/// cache says "this duty is already applied", so without the reset the loop
/// would skip the re-assert and leave fans on the BIOS curve while the UI shows
/// them controlled.
#[cfg(windows)]
fn clear_control_cache(inner: &mut Inner) {
    inner.control_last_commanded.clear();
    inner.control_last_temp.clear();
}

/// Remove the crash-recovery marker on a clean handoff back to the BIOS (STOP,
/// watchdog release, burst/sweep cleanup). A missing marker at next launch means
/// we exited cleanly and left no fan stranded.
#[cfg(windows)]
fn disarm_marker(inner: &mut Inner) {
    if !inner.marker_armed {
        return;
    }
    if let Some(path) = &inner.recovery_marker {
        let _ = std::fs::remove_file(path);
    }
    inner.marker_armed = false;
}

/// Background guard: if the control loop stops refreshing the heartbeat while we
/// hold manual control, hand the fans back to the BIOS so a crashed/frozen UI or
/// a hung controller can never strand a fan at a fixed (possibly low) duty.
/// Spawned once, lives for the process.
#[cfg(windows)]
fn start_watchdog(inner: Arc<Mutex<Inner>>) {
    std::thread::spawn(move || loop {
        std::thread::sleep(Duration::from_secs(1));
        let mut guard = inner.lock();
        if !guard.manual_session_active {
            continue;
        }
        if guard.last_heartbeat.elapsed() <= WATCHDOG_TIMEOUT {
            continue;
        }
        if let Some(chip) = guard.chip.as_ref() {
            let _ = chip.release_to_bios();
        }
        guard.manual_session_active = false;
        disarm_marker(&mut guard);
        clear_control_cache(&mut guard);
    });
}

impl Default for FanSubsystem {
    fn default() -> Self {
        Self::new(
            Arc::new(AtomicBool::new(false)),
            Arc::new(GpuTelemetrySubsystem::new()),
        )
    }
}
