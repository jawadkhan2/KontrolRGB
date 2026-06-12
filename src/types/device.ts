// 1:1 mirrors of the Rust serde types in src-tauri/src/device/types.rs

export interface Color {
  r: number;
  g: number;
  b: number;
}

export type DeviceType = "keyboard" | "motherboard" | "gpu";

export interface KeyInfo {
  led_index: number;
  label: string;
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface ZoneInfo {
  id: string;
  name: string;
  led_count: number;
  resizable: boolean;
  min_leds: number;
  max_leds: number;
  keys: KeyInfo[] | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  device_type: DeviceType;
  zones: ZoneInfo[];
  supported_effects: string[];
}

export type EffectConfig =
  | { kind: "static"; color: Color }
  | { kind: "breathing"; color: Color; speed: number }
  | { kind: "rainbow_wave"; speed: number; reverse: boolean }
  | { kind: "color_cycle"; speed: number }
  | { kind: "custom" };

export type EffectKind = EffectConfig["kind"];

export interface DeviceState {
  effect: EffectConfig;
  brightness: number;
  customColors: Record<string, Color[]>;
}

/** Live animation frame for one device: zone id -> colors. */
export type DeviceFrame = Record<string, Color[]>;

export const cssColor = (c: Color) => `rgb(${c.r}, ${c.g}, ${c.b})`;
