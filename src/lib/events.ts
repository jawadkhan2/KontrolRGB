import { listen } from "@tauri-apps/api/event";
import { useDevices } from "../store/devices";
import type { Color, DeviceInfo } from "../types/device";

interface FramePayload {
  deviceId: string;
  zones: Record<string, Color[]>;
}

/** Wire backend events into the store. Returns an unsubscribe fn. */
export function startEventListeners(): () => void {
  const unsubs: Promise<() => void>[] = [
    listen<FramePayload>("device-frame", (e) => {
      useDevices.getState().updateFrame(e.payload.deviceId, e.payload.zones);
    }),
    listen<DeviceInfo[]>("devices-changed", (e) => {
      useDevices.getState().setDevices(e.payload);
    }),
  ];
  return () => {
    for (const p of unsubs) void p.then((un) => un());
  };
}
