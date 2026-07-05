//! Fan-control safety limits.
//!
//! For the AsiaHorse AMICI-5GT (4-pin PWM, 12V/0.3A) the electrical envelope is
//! fixed by the header hardware — the board holds 12V and you only vary a logic
//! PWM duty, so you physically cannot over-volt or over-current the fan from
//! software. The real failure mode is the *opposite*: too low a duty stalls the
//! fan (it stops spinning), which silently kills airflow and lets components
//! overheat. So the safety limit that matters is a **PWM floor** — never command
//! a duty below the one that reliably keeps the fan turning.
//!
//! These values are conservative defaults. The Phase 2 RPM-sweep tester
//! measures the real stall point per fan and tightens `min_pwm` to the measured
//! value + margin; until then we never let a write go below `DEFAULT_MIN_PWM`.

use serde::{Deserialize, Serialize};

/// Conservative default PWM floor as a percentage. The AMICI-5GT spins down to
/// ~650 RPM; 40% duty keeps it comfortably above stall on a first run before the
/// sweep tester has measured the true floor. Never auto-lowered below this
/// without a measured stall RPM to justify it.
pub const DEFAULT_MIN_PWM: u8 = 40;

/// Datasheet RPM bounds for the AMICI-5GT (120mm: 650–1800). Used to sanity
/// check sweep results and to seed the UI sliders.
#[allow(dead_code)] // Referenced by the sweep sanity checks / UI seeding.
pub const SPEC_MIN_RPM: u16 = 650;
#[allow(dead_code)]
pub const SPEC_MAX_RPM: u16 = 1800;

/// Safety margin (percentage points) added above a measured stall duty before
/// it becomes the new floor. The fan stalled *at* `stall_pct`, so the lowest
/// duty we trust is comfortably above it.
pub const STALL_MARGIN_PCT: u8 = 10;

/// Per-header user/measured limits. Persisted alongside the rest of the config.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FanLimits {
    /// Minimum PWM duty (%) a write may command. Clamped to >= DEFAULT_MIN_PWM
    /// unless a measured stall RPM lowers the safe floor.
    pub min_pwm: u8,
    /// Maximum PWM duty (%) — defaults to 100, user may cap for noise.
    pub max_pwm: u8,
    /// True once the sweep tester has characterized this header. This — not a
    /// sentinel RPM — is what lets the floor drop below `DEFAULT_MIN_PWM`.
    #[serde(default)]
    pub measured: bool,
    /// Slowest RPM the fan still ran at during the sweep (the running speed just
    /// above its stall point). `None` = not yet measured. Display/diagnostics
    /// only; the floor logic keys off `measured`.
    pub measured_stall_rpm: Option<u16>,
    /// Measured top RPM from the sweep tester, if run.
    pub measured_max_rpm: Option<u16>,
}

impl Default for FanLimits {
    fn default() -> Self {
        FanLimits {
            min_pwm: DEFAULT_MIN_PWM,
            max_pwm: 100,
            measured: false,
            measured_stall_rpm: None,
            measured_max_rpm: None,
        }
    }
}

impl FanLimits {
    /// Clamp a requested PWM percentage into the safe window. Phase 2 writes MUST
    /// route through this before reaching the chip.
    pub fn clamp_pct(&self, requested: u8) -> u8 {
        let floor = self.effective_floor();
        let ceiling = self.max_pwm.min(100).max(floor);
        requested.clamp(floor, ceiling)
    }

    /// The floor a clamp actually enforces. A measured stall lets the floor drop
    /// below `DEFAULT_MIN_PWM` (the fan provably runs there); without one, the
    /// conservative default holds.
    pub fn effective_floor(&self) -> u8 {
        if self.measured {
            // Sweep characterized the fan: trust the derived min_pwm as-is.
            self.min_pwm
        } else {
            // No measurement yet: never go below the conservative default.
            self.min_pwm.max(DEFAULT_MIN_PWM)
        }
    }

    /// Fold a sweep result into the limits: the new floor is the measured stall
    /// duty plus a margin (or, if the fan never stalled in range, the lowest
    /// duty it still ran at). Records the measured RPMs so the floor is allowed
    /// to sit below `DEFAULT_MIN_PWM`.
    #[cfg(windows)]
    pub fn apply_sweep(&mut self, sweep: &super::nct6687::SweepResult) {
        let floor = match sweep.stall_pct {
            Some(stall) => stall.saturating_add(STALL_MARGIN_PCT).min(100),
            None => sweep.min_running_pct,
        };
        self.min_pwm = floor.min(self.max_pwm.min(100));
        self.measured = true;
        // Report the slowest RPM the fan still turned at (meaningful non-zero),
        // whether or not it stalled within range — not a 0 sentinel.
        self.measured_stall_rpm = Some(sweep.min_running_rpm);
        self.measured_max_rpm = Some(sweep.max_rpm);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_respects_default_floor() {
        let limits = FanLimits::default();
        assert_eq!(limits.clamp_pct(10), DEFAULT_MIN_PWM);
        assert_eq!(limits.clamp_pct(75), 75);
    }

    #[test]
    fn measured_flag_gates_sub_default_floor() {
        let mut limits = FanLimits {
            min_pwm: 20,
            ..Default::default()
        };
        // Not characterized yet: the conservative default floor still holds even
        // though min_pwm is lower.
        assert_eq!(limits.effective_floor(), DEFAULT_MIN_PWM);
        assert_eq!(limits.clamp_pct(0), DEFAULT_MIN_PWM);
        // Once the sweep has characterized the fan, the lower measured floor is
        // trusted — this is the only path allowed below DEFAULT_MIN_PWM.
        limits.measured = true;
        assert_eq!(limits.effective_floor(), 20);
        assert_eq!(limits.clamp_pct(0), 20);
    }

    #[test]
    fn clamp_does_not_panic_when_max_is_below_floor() {
        let limits = FanLimits {
            min_pwm: 50,
            max_pwm: 30,
            measured: false,
            measured_stall_rpm: None,
            measured_max_rpm: None,
        };
        assert_eq!(limits.clamp_pct(10), 50);
        assert_eq!(limits.clamp_pct(100), 50);
    }
}
