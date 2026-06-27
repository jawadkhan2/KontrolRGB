//! Nuvoton NCT6687D-R driver (MSI Z890 / X870 family Super-I/O).
//!
//! Two layers of access:
//!  1. **SIO config space** (ports 0x2E/0x2F or 0x4E/0x4F): a classic
//!     index/data window used once at probe to read the chip id and the
//!     hardware-monitor I/O base. Nuvoton unlock = write 0x87 twice; lock =
//!     write 0xAA.
//!  2. **EC space** (a paged window at `base+4/5/6`): where the live fan
//!     registers live. Read = select page, select index, read data, release.
//!
//! Register map is ported read-for-read from LibreHardwareMonitor's `Nct677X.cs`
//! NCT6687D branch (verified against the MSI msi_alt1 layout). We do NOT trust
//! the index→header labels blindly: the Z890 relocated the SYS_FAN registers, so
//! the UI mapping wizard makes the user confirm which channel is their case fan
//! by watching live RPM. That empirical confirmation — not these labels — is the
//! source of truth before any write is ever allowed.
//!
//! SAFETY: `read_all` and detection never write fan-control registers. EC reads
//! still touch the EC page/index selector ports, which is part of the chip's
//! read protocol. `release_to_bios` / `release_header` clear manual-mode bits
//! (hand control back to firmware). PHASE 2 writers — `set_manual_pwm`,
//! `discover_pwm_header`, `sweep_header` — spin real fans. They always restore
//! the headers they touched to the BIOS curve when done, and every duty they
//! command must be clamped by the caller (`safety::FanLimits`) first. The one
//! sanctioned exception is `sweep_header`, which deliberately drives below the
//! safety floor to *measure* the real stall point, under continuous tach watch.

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::lpc::Lpc;

/// SIO config index/data port pairs to try, in order.
const SIO_PORTS: [(u16, u16); 2] = [(0x2E, 0x2F), (0x4E, 0x4F)];

/// Nuvoton SIO unlock/lock magic.
const SIO_UNLOCK: u8 = 0x87;
const SIO_LOCK: u8 = 0xAA;

/// SIO registers.
const SIO_CHIP_ID_HI: u8 = 0x20;
const SIO_CHIP_ID_LO: u8 = 0x21;
const SIO_LDN_SELECT: u8 = 0x07;
const SIO_BASE_HI: u8 = 0x60;
const SIO_BASE_LO: u8 = 0x61;

/// Logical device number of the NCT6687 hardware-monitor / EC block.
const LDN_HWM: u8 = 0x0B;

/// The MSI NCT6687D-R boards observed by LibreHardwareMonitor report 0xD592.
/// Do not accept the whole 0xD5xx family here: older NCT6687D boards can share
/// the SIO family id while using different EC layouts.
const SUPPORTED_CHIP_IDS: [u16; 1] = [0xD592];

/// Hardware-monitor I/O bases are decoded ISA I/O windows. Reject obviously
/// bogus values before touching the EC offsets.
const MIN_HWM_BASE: u16 = 0x0100;
const MAX_HWM_BASE: u16 = 0xFFF8;

/// EC paged-window offsets relative to the HM base address.
const EC_PAGE: u16 = 0x04;
const EC_INDEX: u16 = 0x05;
const EC_DATA: u16 = 0x06;
const EC_PAGE_SELECT: u8 = 0xFF;

/// Fan RPM registers (16-bit, big-endian: hi at addr, lo at addr+1). 16 channels.
/// Used by the raw monitoring dump (`read_all`). Control uses `FAN_CTL` instead.
const FAN_RPM_REG: [u16; 16] = [
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x154, 0x156, 0x158,
    0x15A, 0x15C, 0x15E,
];

/// One controllable header's register set.
struct FanCtl {
    /// Tachometer register (which raw RPM channel this header's fan reports on).
    rpm: u16,
    /// PWM duty readback register (0..=255).
    pwm_read: u16,
    /// Manual-mode enable register. CPU/Pump live in `0xA00`; SYS fans in `0x80F`.
    /// Setting `mode_bit` here puts the header in direct-PWM mode.
    mode_reg: u16,
    /// Bit position within `mode_reg` that enables direct-PWM manual control.
    mode_bit: u8,
    /// Direct-PWM command register (single byte, 0..=255). With `mode_bit` set the
    /// EC follows this byte immediately, bypassing the SmartFAN curve engine.
    cmd: u16,
    label: &'static str,
}

/// The 8 controllable headers as wired on this MSI Z890 (Nuvoton NCT6687D-R).
///
/// CONTROL MODEL — direct PWM, ported from LibreHardwareMonitor's `NCT6687DR`
/// branch (the "-R" = the LGA1851/AM5 EC revision this Z890 reports, chip id
/// 0xD592). Every header has a manual-mode bit and a single-byte command
/// register; setting the bit and writing the byte drives the fan IMMEDIATELY,
/// bypassing the EC's SmartFAN curve engine.
///
///  * CPU/Pump: manual bit in `0xA00` (bit = header index), command reg `0xA28`/`0xA29`.
///  * SYS fans: manual bit in `0x80F` (NOT `0xA00`), command reg in page `0x02`
///    (`0x265..0x260`). Duty readback for SYS is at `0xE05..0xE00`.
///
/// HISTORY: SYS fans were previously driven by writing a flat 7-point curve at
/// `0xC10`/`0xC28` etc., because the SYS manual bit was looked for in `0xA00`
/// (wrong register) where it had no effect — so the code fell back to curve mode.
/// The curve engine smooths at ~2%/sec, which was the 20-30s SYS-fan ramp. The
/// fix is `0x80F`: LHM found the SYS manual bits there, enabling true direct PWM.
///
/// User's case fans (empirically confirmed): tach ch11 (`0x156`) → header 6
/// ("System Fan #5", cmd `0x261`, 0x80F bit 3); ch12 (`0x158`) → header 5
/// ("System Fan #4", cmd `0x262`, 0x80F bit 4).
const FAN_CTL: [FanCtl; 8] = [
    FanCtl { rpm: 0x140, pwm_read: 0x160, mode_reg: 0xA00, mode_bit: 0, cmd: 0xA28, label: "CPU Fan" },
    FanCtl { rpm: 0x142, pwm_read: 0x161, mode_reg: 0xA00, mode_bit: 1, cmd: 0xA29, label: "Pump Fan" },
    FanCtl { rpm: 0x15E, pwm_read: 0xE05, mode_reg: 0x80F, mode_bit: 7, cmd: 0x265, label: "System Fan #1" },
    FanCtl { rpm: 0x15C, pwm_read: 0xE04, mode_reg: 0x80F, mode_bit: 6, cmd: 0x264, label: "System Fan #2" },
    FanCtl { rpm: 0x15A, pwm_read: 0xE03, mode_reg: 0x80F, mode_bit: 5, cmd: 0x263, label: "System Fan #3" },
    FanCtl { rpm: 0x158, pwm_read: 0xE02, mode_reg: 0x80F, mode_bit: 4, cmd: 0x262, label: "System Fan #4" },
    FanCtl { rpm: 0x156, pwm_read: 0xE01, mode_reg: 0x80F, mode_bit: 3, cmd: 0x261, label: "System Fan #5" },
    FanCtl { rpm: 0x154, pwm_read: 0xE00, mode_reg: 0x80F, mode_bit: 2, cmd: 0x260, label: "System Fan #6" },
];

/// Config request/commit register: write `FAN_CFG_REQ` to gain access to the
/// fan-control registers, `FAN_CFG_REQ | FAN_CFG_DONE` to commit. Not per-header.
const FAN_PWM_REQUEST_REG: u16 = 0xA01;
/// Fan engine status — polled during the request/commit handshake.
const FAN_ENGINE_STS_REG: u16 = 0xCF8;
const FAN_CFG_REQ: u8 = 0x80;
const FAN_CFG_DONE: u8 = 0x40;
const FAN_CFG_LOCK: u8 = 1 << 6;
const FAN_CFG_PHASE: u8 = 1 << 3;
const FAN_CFG_CHECK_DONE: u8 = 1 << 5;
/// EC sets this in the engine-status register when it rejects a committed
/// configuration. The caller retries the start/write/commit cycle on it.
const FAN_CFG_INVALID: u8 = 1 << 4;

/// Hardware-monitor config register. Bit 0x80 starts the HW monitor / fan
/// engine. The driver's `nct6687_init_device` sets it at probe if clear.
const NCT6687_HWM_CFG: u16 = 0x180;

/// Per-header labels, exposed for the mapping snapshot. The mapping wizard still
/// confirms the real channel empirically — these are display hints.
pub const HEADER_LABELS: [&str; 8] = [
    FAN_CTL[0].label,
    FAN_CTL[1].label,
    FAN_CTL[2].label,
    FAN_CTL[3].label,
    FAN_CTL[4].label,
    FAN_CTL[5].label,
    FAN_CTL[6].label,
    FAN_CTL[7].label,
];

/// Temperature sensor EC addresses for the NCT6687D, base of the page-1
/// hardware-monitor block: temps at `0x100 + i*2`, voltages at `0x120 + i*2`,
/// fan tachs at `0x140 + i*2` (see `FAN_RPM_REG`). Each temp is a 16-bit value:
/// the high byte (at `addr`) is signed Celsius, and bit 7 of the low byte (at
/// `addr+1`) is the 0.5° fraction. Decoded in `read_temps`.
///
/// HISTORY: these were wrongly `0x010..0x014` (page 0) — the page nibble had
/// been dropped (`0x012` instead of `0x102`). Those page-0 bytes are static
/// config, so "CPU" was frozen (e.g. stuck at 47°C) and several degrees off the
/// board's own readout. The stride is the same NCT6687 layout the fan tachs
/// already use, and matches LibreHardwareMonitor / Fred78290's nct6687d driver.
///
/// Source order (CPU, System, VRM, PCH, CPU Socket) is the MSI NCT6687D-R
/// mapping; the user confirms "CPU" tracks their motherboard readout. Keys MUST
/// match the frontend `TempSourceKey` vocabulary (`cpu`, `aux0..aux3`) so the
/// built-in profiles and the temp-source picker resolve.
const TEMP_REGS: &[(u16, &str, &str)] = &[
    (0x100, "cpu", "CPU"),
    (0x102, "aux0", "System"),
    (0x104, "aux1", "VRM MOS"),
    (0x106, "aux2", "PCH"),
    (0x108, "aux3", "CPU Socket"),
];

/// Anything beyond this is almost certainly an unconnected sensor, a bad EC
/// window, or a different board layout. Pumps still sit comfortably below it.
const MAX_REASONABLE_RPM: u16 = 10_000;

#[derive(Debug, thiserror::Error)]
pub enum ChipError {
    #[error("no supported Nuvoton NCT6687 Super-I/O found")]
    NotFound,
    #[error("unsupported NCT6687-family chip id 0x{0:04x}")]
    UnexpectedChipId(u16),
    #[error("invalid hardware-monitor base 0x{0:04x}")]
    InvalidBase(u16),
    #[error("EC window did not respond (read 0x{0:04x})")]
    EcUnresponsive(u16),
    #[error("EC window did not expose plausible live fan telemetry")]
    NoPlausibleTelemetry,
    #[error("fan configuration window did not unlock")]
    FanConfigUnavailable,
    #[error("fan configuration commit did not complete")]
    FanConfigCommitTimeout,
    #[error("EC rejected the committed fan configuration")]
    FanConfigRejected,
    #[error("sweep cancelled by user")]
    SweepCancelled,
}

/// One channel's live reading.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelReading {
    /// Channel index (0..16). 0..8 are the controllable headers in `HEADER_LABELS`.
    pub index: u8,
    /// Best-effort label (controllable headers only; "RPM ch N" beyond 8).
    pub label: String,
    /// Measured tachometer RPM (0 = stopped or no fan connected).
    pub rpm: u16,
    /// Current PWM duty readback as a percentage (None for channels >= 8).
    pub pwm_pct: Option<u8>,
    /// True if this header currently has its manual-mode bit set (we, or BIOS,
    /// took manual control). Channels >= 8 report None.
    pub manual: Option<bool>,
}

/// Outcome of an RPM sweep on one mapped fan: the measured top RPM at full
/// duty, plus the lowest duty that still kept the fan turning (and the duty at
/// which it stalled, if it stalled within the swept range). Drives the safety
/// floor and the UI slider bounds.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SweepResult {
    /// RPM measured at 100% duty.
    pub max_rpm: u16,
    /// Lowest swept duty (%) that still produced a non-zero RPM.
    pub min_running_pct: u8,
    /// RPM at `min_running_pct`.
    pub min_running_rpm: u16,
    /// Duty (%) at which the fan stalled (rpm hit 0), if it stalled in range.
    pub stall_pct: Option<u8>,
    /// Every (duty %, rpm) sample taken, high→low, for display/diagnostics.
    pub samples: Vec<(u8, u16)>,
}

/// One temperature sensor reading from the NCT6687D EC.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TempReading {
    /// Short key used by the frontend to identify the source (e.g. "cpu", "sys").
    pub key: String,
    /// Human-readable label (e.g. "CPU", "System").
    pub label: String,
    /// Temperature in °C, 0.5° resolution. Only returned for sensors in the
    /// plausible range 1–110.
    pub temp_c: f32,
}

/// Which control header drives a given raw tach channel, per the verified
/// msi_alt register map. The confirmed tach channel's RPM register
/// (`FAN_RPM_REG[channel]`) is matched against each header's `rpm` register;
/// e.g. ch11 → 0x156 → header 6 (System Fan #5), ch12 → 0x158 → header 5. Returns
/// None for a tach channel with no controllable header (e.g. a temperature-only
/// or unmapped channel). This is a pure lookup — no hardware access, no spinning.
pub fn header_for_rpm_channel(channel: usize) -> Option<usize> {
    let reg = *FAN_RPM_REG.get(channel)?;
    FAN_CTL.iter().position(|c| c.rpm == reg)
}

/// Inverse of `header_for_rpm_channel`: the raw tach channel that reports a
/// given control header's RPM. Pure lookup. Used by burst auto-detection to
/// know which tach to watch after driving a header to full duty.
pub fn rpm_channel_for_header(header: usize) -> Option<usize> {
    let reg = FAN_CTL.get(header)?.rpm;
    FAN_RPM_REG.iter().position(|&r| r == reg)
}

pub struct Nct6687 {
    lpc: Lpc,
    /// EC window base address read from the SIO at probe.
    base: u16,
    /// Raw 16-bit chip id (e.g. 0xD592 for NCT6687D), for display/diagnostics.
    pub chip_id: u16,
    /// Per-header BIOS state captured the first time we take manual control, so
    /// STOP can restore it. `None` = we have not taken control of that header.
    /// Behind a `Mutex` because chip methods take `&self`; access is already
    /// serialized by the subsystem lock.
    saved: Mutex<[Option<SavedFan>; 8]>,
}

/// A header's BIOS fan state, captured before our first write. The command
/// register holds the live duty, so to truly release we restore the original
/// byte AND the original manual-mode bit, not just clear the bit.
#[derive(Clone, Copy)]
struct SavedFan {
    /// Original manual-mode bit value, masked at the header's bit position within
    /// its `mode_reg` (`0xA00` for CPU/Pump, `0x80F` for SYS).
    mode_bit: u8,
    /// Original byte at the header's `cmd` register.
    duty: u8,
}

fn is_valid_hwm_base(base: u16) -> bool {
    (MIN_HWM_BASE..=MAX_HWM_BASE).contains(&base) && base & 0x0007 == 0
}

fn decode_rpm(raw: u16) -> u16 {
    if raw == 0 || raw == 0xFFFF || raw > MAX_REASONABLE_RPM {
        0
    } else {
        raw
    }
}

impl Nct6687 {
    // --- SIO config-space helpers (used only at probe) ---

    fn sio_read(&self, idx_port: u16, data_port: u16, reg: u8) -> u8 {
        self.lpc.outb(idx_port, reg);
        self.lpc.inb(data_port)
    }

    /// Probe the SIO ports for an NCT6687-family chip and read its HM base.
    pub fn detect(lpc: Lpc) -> Result<Nct6687, ChipError> {
        let mut unsupported_chip = None;
        let mut invalid_base = None;
        let mut ec_error = None;

        for (idx, data) in SIO_PORTS {
            // Unlock: write the magic twice to the index port.
            lpc.outb(idx, SIO_UNLOCK);
            lpc.outb(idx, SIO_UNLOCK);

            let probe = Nct6687 {
                lpc: lpc.clone(),
                base: 0,
                chip_id: 0,
                saved: Mutex::new([None; 8]),
            };
            let hi = probe.sio_read(idx, data, SIO_CHIP_ID_HI);
            let lo = probe.sio_read(idx, data, SIO_CHIP_ID_LO);
            let chip_id = ((hi as u16) << 8) | lo as u16;

            if !SUPPORTED_CHIP_IDS.contains(&chip_id) {
                if hi == 0xD5 || (chip_id & 0xFFF0) == 0xD590 {
                    unsupported_chip = Some(chip_id);
                }
                lpc.outb(idx, SIO_LOCK);
                continue;
            }

            // Select the HM logical device and read its I/O base.
            lpc.outb(idx, SIO_LDN_SELECT);
            lpc.outb(data, LDN_HWM);
            let base_hi = probe.sio_read(idx, data, SIO_BASE_HI);
            let base_lo = probe.sio_read(idx, data, SIO_BASE_LO);
            let base = ((base_hi as u16) << 8) | base_lo as u16;

            // Lock the SIO back before touching the EC window.
            lpc.outb(idx, SIO_LOCK);

            if base == 0 || base == 0xFFFF {
                invalid_base = Some(base);
                continue;
            }
            if !is_valid_hwm_base(base) {
                invalid_base = Some(base);
                continue;
            }

            let chip = Nct6687 {
                lpc: lpc.clone(),
                base,
                chip_id,
                saved: Mutex::new([None; 8]),
            };
            // Validate EC access by requiring at least one plausible live fan
            // tach reading. A blind EC byte read always returns something, so a
            // single "success" result is not enough proof that the window is
            // the NCT6687D-R hardware monitor.
            match chip.validate_ec_window() {
                Ok(_) => {
                    chip.init_device();
                    return Ok(chip);
                }
                Err(e) => {
                    ec_error = Some(e);
                    continue;
                }
            }
        }
        if let Some(e) = ec_error {
            Err(e)
        } else if let Some(base) = invalid_base {
            Err(ChipError::InvalidBase(base))
        } else if let Some(chip_id) = unsupported_chip {
            Err(ChipError::UnexpectedChipId(chip_id))
        } else {
            Err(ChipError::NotFound)
        }
    }

    /// One-time hardware-monitor init, ported from the nct6687d driver's
    /// `nct6687_init_device`: ensure the HW monitor / fan engine is running
    /// (`HWM_CFG` bit 0x80) and enable the SIO voltage inputs. Best-effort —
    /// errors are ignored. CPU control already works without this, so it's
    /// belt-and-suspenders aimed at the SYS-fan engine.
    fn init_device(&self) {
        if let Ok(cfg) = self.ec_read(NCT6687_HWM_CFG) {
            if cfg & 0x80 == 0 {
                eprintln!("[fan init] HWM_CFG was 0x{cfg:02X}, starting HW monitor");
                let _ = self.ec_write(NCT6687_HWM_CFG, cfg | 0x80);
            }
        }
        // Enable SIO voltage inputs (driver parity; harmless for fan control).
        for (addr, val) in [
            (0x1BBu16, 0x61u8),
            (0x1BC, 0x62),
            (0x1BD, 0x63),
            (0x1BE, 0x64),
            (0x1BF, 0x65),
        ] {
            let _ = self.ec_write(addr, val);
        }
    }

    fn validate_ec_window(&self) -> Result<(), ChipError> {
        let mut saw_non_ff = false;
        let mut saw_non_zero = false;
        let mut plausible_rpm = false;

        for &reg in &FAN_RPM_REG {
            let raw = self.ec_read_u16(reg)?;
            saw_non_ff |= raw != 0xFFFF;
            saw_non_zero |= raw != 0;
            plausible_rpm |= decode_rpm(raw) > 0;
        }

        if !saw_non_ff {
            return Err(ChipError::EcUnresponsive(0xFFFF));
        }
        if !saw_non_zero {
            return Err(ChipError::EcUnresponsive(0x0000));
        }
        if !plausible_rpm {
            return Err(ChipError::NoPlausibleTelemetry);
        }

        Ok(())
    }

    // --- EC paged-window access ---

    /// Read one byte from EC space. Side-effect free w.r.t. fan state.
    fn ec_read(&self, addr: u16) -> Result<u8, ChipError> {
        let page = (addr >> 8) as u8;
        let index = (addr & 0xFF) as u8;

        // Wait for the page register to be free (reads back 0xFF), max 500ms;
        // if it never frees, force it (matches LibreHardwareMonitor).
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.lpc.inb(self.base + EC_PAGE) != EC_PAGE_SELECT {
            if Instant::now() >= deadline {
                self.lpc.outb(self.base + EC_PAGE, EC_PAGE_SELECT);
                break;
            }
        }

        self.lpc.outb(self.base + EC_PAGE, page);
        self.lpc.outb(self.base + EC_INDEX, index);
        let data = self.lpc.inb(self.base + EC_DATA);
        // Release the window so other readers (and BIOS) can use it.
        self.lpc.outb(self.base + EC_PAGE, EC_PAGE_SELECT);
        Ok(data)
    }

    fn ec_read_u16(&self, addr: u16) -> Result<u16, ChipError> {
        let hi = self.ec_read(addr)? as u16;
        let lo = self.ec_read(addr + 1)? as u16;
        Ok((hi << 8) | lo)
    }

    /// Read every channel: RPM for all 16, PWM-out + manual flag for the 8
    /// controllable headers. Pure read — never mutates fan state.
    pub fn read_all(&self) -> Result<Vec<ChannelReading>, ChipError> {
        let mut out = Vec::with_capacity(FAN_RPM_REG.len());
        for (i, &reg) in FAN_RPM_REG.iter().enumerate() {
            // 16-channel tach dump for monitoring (the index is the tach position
            // the user confirms). Where a tach channel corresponds to a control
            // header under the msi_alt map, show that header's real label, duty
            // and manual flag; other positions are tach-only.
            let rpm = decode_rpm(self.ec_read_u16(reg)?);
            let (label, pwm_pct, manual) = match header_for_rpm_channel(i) {
                Some(h) => {
                    let ctl = &FAN_CTL[h];
                    let duty = self.ec_read(ctl.pwm_read)?;
                    let pct = ((duty as u16 * 100) / 255) as u8;
                    let man = (self.ec_read(ctl.mode_reg)? >> ctl.mode_bit) & 1 == 1;
                    (ctl.label.to_string(), Some(pct), Some(man))
                }
                None => (format!("RPM ch {i}"), None, None),
            };
            out.push(ChannelReading {
                index: i as u8,
                label,
                rpm,
                pwm_pct,
                manual,
            });
        }
        Ok(out)
    }

    /// Read all temperature sensors. Returns only sensors with plausible values
    /// (1..=110 °C); unconnected or invalid sensors are silently omitted.
    pub fn read_temps(&self) -> Result<Vec<TempReading>, ChipError> {
        let mut out = Vec::new();
        for &(addr, key, label) in TEMP_REGS {
            // 16-bit: hi byte = signed °C, bit 7 of lo byte = 0.5° fraction.
            let hi = self.ec_read(addr)? as i8;
            let lo = self.ec_read(addr + 1)?;
            let temp = hi as f32 + if lo & 0x80 != 0 { 0.5 } else { 0.0 };
            if (1.0..=110.0).contains(&temp) {
                out.push(TempReading {
                    key: key.to_string(),
                    label: label.to_string(),
                    temp_c: temp,
                });
            }
        }
        Ok(out)
    }

    /// Hand every header we took back to the BIOS fan curve.
    pub fn release_to_bios(&self) -> Result<(), ChipError> {
        for index in 0..FAN_CTL.len() {
            self.release_header(index)?;
        }
        Ok(())
    }

    /// Write one byte to EC space. PRIVATE: the only public writers are
    /// `release_to_bios` and, later, `set_manual_pwm` (Phase 2).
    fn ec_write(&self, addr: u16, value: u8) -> Result<(), ChipError> {
        let page = (addr >> 8) as u8;
        let index = (addr & 0xFF) as u8;
        let deadline = Instant::now() + Duration::from_millis(500);
        while self.lpc.inb(self.base + EC_PAGE) != EC_PAGE_SELECT {
            if Instant::now() >= deadline {
                self.lpc.outb(self.base + EC_PAGE, EC_PAGE_SELECT);
                break;
            }
        }
        self.lpc.outb(self.base + EC_PAGE, page);
        self.lpc.outb(self.base + EC_INDEX, index);
        self.lpc.outb(self.base + EC_DATA, value);
        self.lpc.outb(self.base + EC_PAGE, EC_PAGE_SELECT);
        Ok(())
    }

    /// Write a commanded duty (0..=255) to one header's direct-PWM command
    /// register. Uniform across CPU/Pump and SYS now that the SYS manual bit
    /// (`0x80F`) selects direct PWM. Caller holds the manual bit and wraps this in
    /// the `start_fan_cfg`/`finish_fan_cfg` handshake (see `commit_duty`).
    fn write_duty(&self, index: usize, duty: u8) -> Result<(), ChipError> {
        self.ec_write(FAN_CTL[index].cmd, duty)
    }

    /// Read a broad swath of EC registers across the fan/HWM pages, for passive
    /// capture/diffing — e.g. snapshotting before/after an external tool (MSI
    /// Center) moves a fan, to discover the registers it actually writes. Pure
    /// read; never mutates fan state.
    pub fn scan_ec(&self) -> Vec<(u16, u8)> {
        // Full EC address space (all 256 pages) so a config register MSI Center
        // writes anywhere — e.g. a fan step-up / response time — is captured.
        let mut out = Vec::with_capacity(0x10000);
        for page in 0..=0xFFu16 {
            for i in 0..=0xFFu16 {
                let addr = (page << 8) | i;
                if let Ok(v) = self.ec_read(addr) {
                    out.push((addr, v));
                }
            }
        }
        out
    }

    /// Read one channel's tach RPM. Pure read.
    pub fn read_rpm(&self, channel: usize) -> Result<u16, ChipError> {
        let reg = *FAN_RPM_REG
            .get(channel)
            .ok_or(ChipError::EcUnresponsive(0))?;
        Ok(decode_rpm(self.ec_read_u16(reg)?))
    }

    /// Capture a header's BIOS state — the manual-mode bit AND the original
    /// `pwm_write` command byte — the first time we take control of it. MUST run
    /// before our first write, because that write overwrites the duty we need to
    /// restore later. Idempotent.
    fn save_initial(&self, index: usize) -> Result<(), ChipError> {
        if self.saved.lock()[index].is_some() {
            return Ok(());
        }
        let ctl = &FAN_CTL[index];
        let mode = self.ec_read(ctl.mode_reg)?;
        let mode_bit = mode & (1 << ctl.mode_bit);
        let duty = self.ec_read(ctl.cmd)?;
        eprintln!("[fan restore] saved header{index} mode_bit={mode_bit} duty={duty}");
        self.saved.lock()[index] = Some(SavedFan { mode_bit, duty });
        Ok(())
    }

    /// Return one header to the BIOS: rewrite the ORIGINAL duty byte (our write
    /// clobbered the `pwm_write` command register), restore the original
    /// manual-mode bit, and commit through the config handshake. Clearing the
    /// `0xA00` bit alone does NOT release on Z890. Used by STOP, the watchdog, and
    /// sweep cleanup. No-op for a header we never took.
    pub fn release_header(&self, index: usize) -> Result<(), ChipError> {
        if index >= FAN_CTL.len() {
            return Err(ChipError::EcUnresponsive(0));
        }
        let saved = self.saved.lock()[index];
        let Some(saved) = saved else {
            return Ok(()); // we never took this header — leave it alone
        };
        let ctl = &FAN_CTL[index];
        let mode = self.ec_read(ctl.mode_reg)?;
        let new_mode = (mode & !(1u8 << ctl.mode_bit)) | saved.mode_bit;
        self.ec_write(ctl.mode_reg, new_mode)?;
        // Restore the saved duty through the config handshake. Clearing the mode
        // bit alone does NOT release on Z890 — the change must be committed.
        self.commit_duty(index, saved.duty)?;
        self.saved.lock()[index] = None;
        eprintln!("[fan restore] released header{index}, duty restored");
        Ok(())
    }

    /// Open the fan-config window: request access (`0x80`) and poll the engine
    /// status (`0xCF8`) until the EC unlocks the control registers. Ported from
    /// the nct6687d Linux driver's `start_fan_cfg_update`.
    fn start_fan_cfg(&self) -> Result<(), ChipError> {
        let sts = self.ec_read(FAN_ENGINE_STS_REG)?;
        if sts & FAN_CFG_LOCK == 0 && sts & FAN_CFG_PHASE != 0 {
            return Ok(()); // already accessible
        }
        // Wait until any in-progress config phase is done and the request is clear.
        let mut ready = false;
        for _ in 0..1000 {
            let phase = self.ec_read(FAN_ENGINE_STS_REG)? & FAN_CFG_PHASE;
            let req = self.ec_read(FAN_PWM_REQUEST_REG)? & FAN_CFG_REQ;
            if phase == 0 && req == 0 {
                ready = true;
                break;
            }
            sleep(Duration::from_millis(1));
        }
        if !ready {
            return Err(ChipError::FanConfigUnavailable);
        }
        self.ec_write(FAN_PWM_REQUEST_REG, FAN_CFG_REQ)?;
        sleep(Duration::from_millis(10)); // EC needs a fixed settle after the request
        // Wait until the EC enters config phase and unlocks the register set.
        for _ in 0..1000 {
            let sts = self.ec_read(FAN_ENGINE_STS_REG)?;
            if sts & FAN_CFG_LOCK == 0 && sts & FAN_CFG_PHASE != 0 {
                return Ok(());
            }
            sleep(Duration::from_millis(1));
        }
        Err(ChipError::FanConfigUnavailable)
    }

    /// Commit the fan-config window: signal done (`REQ|DONE`) and poll until the
    /// EC checks the new configuration, returning `FanConfigRejected` if the EC
    /// raises INVALID. Ported from LHM's `CompleteFanConfigUpdate`.
    fn finish_fan_cfg(&self) -> Result<(), ChipError> {
        let sts = self.ec_read(FAN_ENGINE_STS_REG)?;
        if sts & FAN_CFG_LOCK != 0 || sts & FAN_CFG_PHASE == 0 {
            return Ok(()); // already committed/inaccessible
        }
        // Commit atomically with REQ|DONE (0xC0). DONE alone (0x40) commits
        // CPU/Pump, but the SYS-fan direct-PWM path needs the request bit held
        // through the commit or the EC drops it.
        self.ec_write(FAN_PWM_REQUEST_REG, FAN_CFG_REQ | FAN_CFG_DONE)?;
        sleep(Duration::from_millis(10));
        for _ in 0..1000 {
            if self.ec_read(FAN_ENGINE_STS_REG)? & FAN_CFG_CHECK_DONE != 0 {
                break;
            }
            sleep(Duration::from_millis(1));
        }
        // EC raises INVALID if it rejected the configuration; the caller retries.
        if self.ec_read(FAN_ENGINE_STS_REG)? & FAN_CFG_INVALID != 0 {
            return Err(ChipError::FanConfigRejected);
        }
        Ok(())
    }

    /// Open the config window, write `duty` to the header's command register, and
    /// commit — retrying the whole cycle up to 3× if the EC rejects it (INVALID),
    /// mirroring LHM's retry loop. The caller must already hold the manual bit.
    fn commit_duty(&self, index: usize, duty: u8) -> Result<(), ChipError> {
        let mut last_err = ChipError::FanConfigCommitTimeout;
        for _ in 0..3 {
            self.start_fan_cfg()?; // a start failure is fatal — don't retry it
            self.write_duty(index, duty)?;
            match self.finish_fan_cfg() {
                Ok(()) => return Ok(()),
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    /// Take manual control of one header and set its PWM duty (0..=255). The
    /// caller MUST clamp `duty` to a safe floor (`safety::FanLimits::clamp_pct`)
    /// first, except in `sweep_header` which is measuring the floor.
    ///
    /// Sequence (NCT6687D-R direct PWM): SET the header's manual bit (CPU/Pump in
    /// `0xA00`, SYS in `0x80F`), then write the duty through the config handshake
    /// (`commit_duty`). The set bit makes the EC follow the single-byte command
    /// register directly, bypassing the SmartFAN curve engine and its ~2%/sec
    /// smoothing — which was the 20-30s SYS-fan ramp. Logs a duty readback + tach
    /// RPM so a manual test is conclusive.
    pub fn set_manual_pwm(&self, index: usize, duty: u8) -> Result<(), ChipError> {
        let ctl = FAN_CTL.get(index).ok_or(ChipError::EcUnresponsive(0))?;
        // Remember the BIOS state before our first write so STOP can restore it.
        self.save_initial(index)?;
        // Enable direct-PWM manual mode by SETTING this header's manual bit. Every
        // header responds to its command register once the bit is set — the SYS
        // bit lives in 0x80F, not 0xA00, which is why the earlier 0xA00-only path
        // could not drive SYS fans and fell back to slow curve mode.
        let mode = self.ec_read(ctl.mode_reg)?;
        self.ec_write(ctl.mode_reg, mode | (1 << ctl.mode_bit))?;
        self.commit_duty(index, duty)?;
        let readback = self.ec_read(ctl.pwm_read).unwrap_or(0);
        let rpm = decode_rpm(self.ec_read_u16(ctl.rpm).unwrap_or(0));
        eprintln!(
            "[fan set] header{index} cmd={duty} committed=true readback={readback} rpm={rpm}"
        );
        Ok(())
    }

    /// Sweep a mapped header's duty from full down toward zero, recording the
    /// tach RPM at each step, to measure this fan's real top RPM and stall
    /// point. This is the ONE writer allowed below the safety floor — it is how
    /// the floor is discovered. Bounded (stops at the first stall or 5%), under
    /// continuous tach watch, and restores the header to BIOS when finished.
    ///
    /// `on_progress(pct, rpm, phase)` is called for each measured point (and once
    /// with phase `"settling"` while a step's RPM is stabilizing) so the UI can
    /// show live duty/RPM during the ~30s sweep — the subsystem lock is held for
    /// the whole sweep, so polling can't read it any other way.
    pub fn sweep_header(
        &self,
        header: usize,
        rpm_channel: usize,
        cancel: &AtomicBool,
        on_progress: &dyn Fn(u8, u16, &'static str),
    ) -> Result<SweepResult, ChipError> {
        if header >= FAN_CTL.len() {
            return Err(ChipError::EcUnresponsive(0));
        }
        // Case fans (esp. larger SYS fans) can take up to ~20s to fully settle on
        // a new duty — measuring sooner reads a mid-ramp RPM and, on the way down,
        // misreports a still-spinning-down fan as a stall. So we give each step a
        // long settle. This makes a full sweep take a few minutes per fan; the UI
        // warns the user up front and offers a Stop button.
        const SETTLE: Duration = Duration::from_secs(20);
        const TOP_SETTLE: Duration = Duration::from_secs(20);
        const STEP_PCT: i32 = 5;
        const FLOOR_PCT: i32 = 5;

        let pct_to_duty = |pct: i32| ((pct.clamp(0, 100) as u16 * 255) / 100) as u8;

        // Sleep in short slices so a Stop request is honoured within ~200ms even
        // mid-settle. On cancel, hand the header back to the BIOS and bail. We
        // also read the tach about once a second and emit it as a "settling"
        // sample so the UI shows the fan actually ramping (the subsystem lock is
        // held for the whole sweep, so this is the only place a live RPM exists).
        let settle = |dur: Duration, pct: u8| -> Result<(), ChipError> {
            let deadline = Instant::now() + dur;
            let mut next_emit = Instant::now();
            while Instant::now() < deadline {
                if cancel.load(Ordering::Relaxed) {
                    let _ = self.release_header(header);
                    return Err(ChipError::SweepCancelled);
                }
                if Instant::now() >= next_emit {
                    if let Ok(rpm) = self.read_rpm(rpm_channel) {
                        on_progress(pct, rpm, "settling");
                    }
                    next_emit = Instant::now() + Duration::from_millis(1000);
                }
                sleep(Duration::from_millis(200));
            }
            Ok(())
        };

        // Top end: full duty.
        on_progress(100, 0, "settling");
        self.set_manual_pwm(header, 255)?;
        settle(TOP_SETTLE, 100)?;
        let max_rpm = self.read_rpm(rpm_channel)?;
        on_progress(100, max_rpm, "measuring");

        let mut samples = vec![(100u8, max_rpm)];
        let mut min_running_pct = 100u8;
        let mut min_running_rpm = max_rpm;
        let mut stall_pct = None;

        // Start the descent just below the conservative floor and walk down.
        let mut pct = 50i32;
        while pct >= FLOOR_PCT {
            on_progress(pct as u8, 0, "settling");
            self.set_manual_pwm(header, pct_to_duty(pct))?;
            settle(SETTLE, pct as u8)?;
            let rpm = self.read_rpm(rpm_channel)?;
            on_progress(pct as u8, rpm, "measuring");
            samples.push((pct as u8, rpm));
            if rpm == 0 {
                stall_pct = Some(pct as u8);
                break;
            }
            min_running_pct = pct as u8;
            min_running_rpm = rpm;
            pct -= STEP_PCT;
        }

        self.release_header(header)?;
        Ok(SweepResult {
            max_rpm,
            min_running_pct,
            min_running_rpm,
            stall_pct,
            samples,
        })
    }
}
