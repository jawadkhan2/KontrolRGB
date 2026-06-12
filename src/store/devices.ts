import { create } from "zustand";
import * as api from "../lib/api";
import type {
  Color,
  DeviceFrame,
  DeviceInfo,
  DeviceState,
  EffectConfig,
} from "../types/device";

interface DevicesStore {
  devices: DeviceInfo[];
  selectedId: string | null;
  states: Record<string, DeviceState>;
  frames: Record<string, DeviceFrame>;
  /** Color used when clicking keys/LEDs to paint them. */
  paintColor: Color;

  init: () => Promise<void>;
  select: (id: string) => void;
  setDevices: (devices: DeviceInfo[]) => void;
  updateFrame: (deviceId: string, frame: DeviceFrame) => void;
  setPaintColor: (color: Color) => void;

  applyEffect: (deviceId: string, effect: EffectConfig) => void;
  applyBrightness: (deviceId: string, brightness: number) => void;
  paintLed: (deviceId: string, zoneId: string, ledIndex: number) => void;
  resizeZone: (deviceId: string, zoneId: string, ledCount: number) => void;
  applyToAll: (effect: EffectConfig, brightness?: number) => void;
  rescan: () => void;
}

async function fetchStates(devices: DeviceInfo[]) {
  const entries = await Promise.all(
    devices.map(async (d) => [d.id, await api.getDeviceState(d.id)] as const),
  );
  return Object.fromEntries(entries) as Record<string, DeviceState>;
}

export const useDevices = create<DevicesStore>((set, get) => ({
  devices: [],
  selectedId: null,
  states: {},
  frames: {},
  paintColor: { r: 139, g: 92, b: 246 },

  init: async () => {
    const devices = await api.listDevices();
    const states = await fetchStates(devices);
    set((s) => ({
      devices,
      states,
      selectedId: s.selectedId ?? devices[0]?.id ?? null,
    }));
  },

  select: (id) => set({ selectedId: id }),

  setDevices: (devices) => {
    set((s) => ({
      devices,
      selectedId:
        s.selectedId && devices.some((d) => d.id === s.selectedId)
          ? s.selectedId
          : devices[0]?.id ?? null,
    }));
    void fetchStates(devices).then((states) => set({ states }));
  },

  updateFrame: (deviceId, frame) =>
    set((s) => ({ frames: { ...s.frames, [deviceId]: frame } })),

  setPaintColor: (paintColor) => set({ paintColor }),

  applyEffect: (deviceId, effect) => {
    set((s) => ({
      states: {
        ...s.states,
        [deviceId]: { ...s.states[deviceId], effect },
      },
    }));
    void api.setEffect(deviceId, effect);
  },

  applyBrightness: (deviceId, brightness) => {
    set((s) => ({
      states: {
        ...s.states,
        [deviceId]: { ...s.states[deviceId], brightness },
      },
    }));
    void api.setBrightness(deviceId, brightness);
  },

  paintLed: (deviceId, zoneId, ledIndex) => {
    const { paintColor, devices, states } = get();
    const zone = devices
      .find((d) => d.id === deviceId)
      ?.zones.find((z) => z.id === zoneId);
    if (!zone) return;

    const state = states[deviceId];
    const colors = [...(state?.customColors[zoneId] ?? [])];
    while (colors.length < zone.led_count) colors.push({ r: 0, g: 0, b: 0 });
    colors[ledIndex] = paintColor;

    set((s) => ({
      states: {
        ...s.states,
        [deviceId]: {
          ...state,
          effect: { kind: "custom" },
          customColors: { ...state.customColors, [zoneId]: colors },
        },
      },
    }));
    void api.setLedColor(deviceId, zoneId, ledIndex, paintColor);
  },

  resizeZone: (deviceId, zoneId, ledCount) => {
    void api.resizeZone(deviceId, zoneId, ledCount).then((devices) => {
      set({ devices });
    });
  },

  applyToAll: (effect, brightness) => {
    set((s) => ({
      states: Object.fromEntries(
        Object.entries(s.states).map(([id, st]) => [
          id,
          { ...st, effect, brightness: brightness ?? st.brightness },
        ]),
      ),
    }));
    void api.applyToAll(effect, brightness);
  },

  rescan: () => {
    void api.rescanDevices().then((devices) => get().setDevices(devices));
  },
}));
