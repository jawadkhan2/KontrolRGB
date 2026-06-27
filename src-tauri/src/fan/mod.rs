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

pub const CONTROLLABLE_CHANNEL_COUNT: u8 = 8;
pub const RPM_CHANNEL_COUNT: u8 = 16;
const PUMP_HEADER_INDEX: u8 = 1;

/// If the UI stops sending heartbeats for this long while we hold manual
/// control, the watchdog hands the fans back to the BIOS. The Fan page pings
/// every poll (~1.5s), so this tolerates a couple of missed frames before
/// assuming the UI died.
#[cfg(windows)]
const WATCHDOG_TIMEOUT: Duration = Duration::from_secs(6);

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
}

impl FanSubsystem {
    pub fn new() -> Self {
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
            })),
            sweep_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Request cancellation of an in-flight sweep. Lock-free so it works while the
    /// sweep thread holds the subsystem mutex; the sweep loop releases the header
    /// back to the BIOS and bails on the next slice. No-op if nothing is sweeping.
    pub fn cancel_sweep(&self) {
        self.sweep_cancel.store(true, Ordering::Relaxed);
    }

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

        // Phase 1 — burst every non-pump header to full duty.
        {
            let mut inner = self.inner.lock();
            Self::ensure_open(&mut inner);
            let chip = inner
                .chip
                .as_ref()
                .ok_or_else(|| FanError::Unavailable(inner.detail.clone()))?;
            for &(header, _) in &header_tach {
                chip.set_manual_pwm(header as usize, 255)?;
            }
            inner.manual_session_active = true;
            inner.last_heartbeat = Instant::now();
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
        )?;
        // sweep_header restores the header to BIOS itself (also on cancel, where it
        // returns SweepCancelled and we never touch the limits below).
        inner.manual_session_active = false;
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

    /// UI heartbeat: keep the watchdog from releasing control. The Fan page
    /// calls this every poll while it holds any manual fan. Uses `try_lock` so
    /// it never blocks behind a long hardware op (a missed beat or two is fine —
    /// the watchdog timeout is several poll intervals wide).
    #[cfg(windows)]
    pub fn heartbeat(&self) {
        if let Some(mut inner) = self.inner.try_lock() {
            inner.last_heartbeat = Instant::now();
        }
    }

    #[cfg(not(windows))]
    pub fn heartbeat(&self) {}

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

/// Background guard: if the UI stops sending heartbeats while we hold manual
/// control, hand the fans back to the BIOS so a crashed/frozen/closed UI can
/// never strand a fan at a fixed (possibly low) duty. Spawned once, lives for
/// the process.
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
    });
}

impl Default for FanSubsystem {
    fn default() -> Self {
        Self::new()
    }
}
