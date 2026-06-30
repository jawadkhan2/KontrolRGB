import { invoke } from "@tauri-apps/api/core";

// Fan control (case fans, NCT6687D-R via ring-0 LPC).
//
// Phase 2: speed control. Flow — confirm a tach channel (Phase 1), map it to a
// PWM header, then set a clamped duty. A sweep measures the real stall floor; a
// heartbeat keeps the watchdog from handing the fan back to the BIOS.

/** One mapped, controllable fan and its safe bounds. */
export interface FanMapping {
  rpmChannel: number;
  header: number;
  headerLabel: string;
  /** Effective floor %, may be below the default once a stall is measured. */
  minPwm: number;
  maxPwm: number;
  measuredStallRpm: number | null;
  measuredMaxRpm: number | null;
}

export interface FanStatus {
  available: boolean;
  detail: string;
  chip: string | null;
  confirmedRpmChannels: number[];
  /** Chip detected → writes are possible. */
  writesEnabled: boolean;
  /** KontrolRGB is holding at least one header in manual control. */
  manualActive: boolean;
  mappings: FanMapping[];
}

export interface FanSnapshot {
  status: FanStatus;
  readings: ChannelReading[];
  temps: TempReading[];
}

export interface ChannelReading {
  index: number;
  label: string;
  rpm: number;
  /** PWM duty % readback; null for non-controllable channels (>= 8). */
  pwmPct: number | null;
  /** True if this header is currently under manual control; null for >= 8. */
  manual: boolean | null;
}

/** Live per-sample progress streamed during a sweep (event `fan-sweep-progress`). */
export interface SweepProgress {
  rpmChannel: number;
  pct: number;
  rpm: number;
  /** "settling" while a step stabilizes, "measuring" once the RPM is read. */
  phase: "settling" | "measuring";
}

/** Result of an RPM sweep: top RPM, lowest running duty, and stall point. */
export interface SweepResult {
  maxRpm: number;
  minRunningPct: number;
  minRunningRpm: number;
  /** Duty % where the fan stalled, or null if it never stalled in range. */
  stallPct: number | null;
  /** [dutyPct, rpm] samples, high→low. */
  samples: [number, number][];
}

export const fanStatus = () => invoke<FanStatus>("fan_status");

export const fanSnapshot = () => invoke<FanSnapshot>("fan_snapshot");

export const fanRead = () => invoke<ChannelReading[]>("fan_read");

/** STOP / panic: hand every header back to the BIOS fan curve. */
export const fanStop = () => invoke<void>("fan_stop");

export const fanConfirmChannel = (index: number) =>
  invoke<void>("fan_confirm_channel", { index });

/** Discover which PWM header drives a tach channel. Spins fans; ~10s. */
export const fanMapHeader = (rpmChannel: number) =>
  invoke<number>("fan_map_header", { rpmChannel });

/** Result of a burst auto-detect: tach channels found spinning and auto-mapped. */
export interface BurstResult {
  detected: number[];
}

/** Live per-header stats for one burst sample (event `fan-burst-progress`). */
export interface BurstFanProgress {
  header: number;
  headerLabel: string;
  rpmChannel: number;
  /** RPM read this sample. */
  rpm: number;
  /** Highest RPM seen so far this run. */
  maxRpm: number;
  /** A fan is spinning on this header right now (rpm > 0). */
  detected: boolean;
}

/** One burst sample snapshot streamed during auto-detect. */
export interface BurstProgress {
  elapsedMs: number;
  /** Total burst window in ms (configured hold), for a countdown. */
  totalMs: number;
  /** "bursting" | "done". */
  phase: string;
  fans: BurstFanProgress[];
}

/**
 * Burst auto-detect: drive every controllable header (except the pump) to 100%,
 * hold for `durationSecs`, then map every header still spinning at the end as a
 * controllable fan. Pump stays on BIOS.
 */
export const fanBurstDetect = (durationSecs: number) =>
  invoke<BurstResult>("fan_burst_detect", { durationSecs });

/** Set a mapped fan's duty (%). Returns the clamped % actually applied. */
export const fanSetSpeed = (rpmChannel: number, pct: number) =>
  invoke<number>("fan_set_speed", { rpmChannel, pct });

/**
 * Sweep a mapped fan to measure its stall floor and top RPM. Long-running: each
 * duty step waits ~20s for the fan to fully settle, so a full sweep is a few
 * minutes per fan. Restores the fan to the BIOS when done.
 */
export const fanSweep = (rpmChannel: number) =>
  invoke<SweepResult>("fan_sweep", { rpmChannel });

/** Cancel an in-flight sweep (Stop button). Fan is handed back to the BIOS. */
export const fanCancelSweep = () => invoke<void>("fan_cancel_sweep");

/** Keep the watchdog from releasing control while a fan is held. */
export const fanHeartbeat = () => invoke<void>("fan_heartbeat");

// --- Background control plan ------------------------------------------------

/** One fan-curve point, in the units the backend interpolates. */
export interface ControlCurvePoint {
  tempC: number;
  speedPct: number;
}

/** How a single fan is driven by the backend control loop. */
export type ControlMode =
  | { type: "manual"; pct: number }
  | { type: "curve"; tempSource: string; points: ControlCurvePoint[] };

/** Full background control plan handed to the backend control loop. */
export interface FanControlPlan {
  /** STOP → BIOS: the loop asserts nothing while true. */
  released: boolean;
  /** Mode for mapped fans with no explicit entry (e.g. burst-detected). */
  defaultMode: ControlMode | null;
  /** Per tach-channel mode. */
  modes: Record<number, ControlMode>;
}

/**
 * Install the background control plan. The backend's in-process loop then holds
 * every mapped fan to its target every couple of seconds — immune to webview
 * timer throttling when the window is hidden, unlike the old JS-driven loop.
 */
export const fanSetControlPlan = (plan: FanControlPlan) =>
  invoke<void>("fan_set_control_plan", { plan });

export interface TempReading {
  key: string;
  label: string;
  tempC: number;
}

/** Read motherboard temperature sensors (NCT6687D EC). Only connected sensors returned. */
export const fanReadTemps = () => invoke<TempReading[]>("fan_read_temps");
