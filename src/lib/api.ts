import { invoke } from "@tauri-apps/api/core";
import type { Color, DeviceInfo, DeviceState, EffectConfig } from "../types/device";

// Note: Rust snake_case args are passed camelCased per Tauri 2 convention.

export const listDevices = () => invoke<DeviceInfo[]>("list_devices");

export const getDeviceState = (deviceId: string) =>
  invoke<DeviceState>("get_device_state", { deviceId });

export const setEffect = (deviceId: string, effect: EffectConfig) =>
  invoke<void>("set_effect", { deviceId, effect });

export const setBrightness = (deviceId: string, brightness: number) =>
  invoke<void>("set_brightness", { deviceId, brightness });

export const setZoneColors = (deviceId: string, zoneId: string, colors: Color[]) =>
  invoke<void>("set_zone_colors", { deviceId, zoneId, colors });

export const setLedColor = (
  deviceId: string,
  zoneId: string,
  ledIndex: number,
  color: Color,
) => invoke<void>("set_led_color", { deviceId, zoneId, ledIndex, color });

export const resizeZone = (deviceId: string, zoneId: string, ledCount: number) =>
  invoke<DeviceInfo[]>("resize_zone", { deviceId, zoneId, ledCount });

export const renameZone = (deviceId: string, zoneId: string, name: string) =>
  invoke<DeviceInfo[]>("rename_zone", { deviceId, zoneId, name });

export const identifyZone = (deviceId: string, zoneId: string) =>
  invoke<void>("identify_zone", { deviceId, zoneId });

export const rescanDevices = () => invoke<DeviceInfo[]>("rescan_devices");

// --- GPU telemetry (NVML) -------------------------------------------------

/** Live GPU telemetry. Each metric is null when the driver won't report it. */
export interface GpuTelemetry {
  /** NVML loaded and a GPU bound. When false every metric is null. */
  available: boolean;
  name: string | null;
  tempC: number | null;
  coreClockMhz: number | null;
  fanPct: number | null;
  powerW: number | null;
}

export const gpuTelemetry = () => invoke<GpuTelemetry>("gpu_telemetry");

// --- Startup conflict guard -----------------------------------------------

export interface ConflictProcess {
  pid: number;
  displayName: string;
  exeName: string;
}

export const scanRgbConflicts = () =>
  invoke<ConflictProcess[]>("scan_rgb_conflicts");

export const killRgbConflicts = (pids: number[]) =>
  invoke<void>("kill_rgb_conflicts", { pids });

export const quitApp = () => invoke<void>("quit_app");

// --- Administrator elevation ----------------------------------------------

/** Whether the running process already has administrator rights. */
export const isElevated = () => invoke<boolean>("is_elevated");

/** Relaunch elevated (pops UAC). On success the current process exits, so the
 *  promise only ever resolves/rejects when elevation was declined or failed. */
export const relaunchAsAdmin = () => invoke<void>("relaunch_as_admin");
