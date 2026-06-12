import { useState } from "react";
import { useDevices } from "../store/devices";
import type { DeviceInfo, DeviceType } from "../types/device";
import { cssColor } from "../types/device";
import { EffectPanel } from "./effects/EffectPanel";

const TYPE_ICONS: Record<DeviceType, string> = {
  keyboard: "⌨️",
  motherboard: "🖥️",
  gpu: "🎮",
};

function LiveSwatch({ device }: { device: DeviceInfo }) {
  const frame = useDevices((s) => s.frames[device.id]);
  const firstZone = device.zones[0];
  const colors = frame?.[firstZone?.id] ?? [];
  // Sample a few LEDs across the zone for a mini gradient.
  const samples = [0, 0.25, 0.5, 0.75, 1].map((p) => {
    const c = colors[Math.floor(p * Math.max(0, colors.length - 1))];
    return c ? cssColor(c) : "#27272a";
  });
  return (
    <div
      className="h-2.5 w-16 rounded-full"
      style={{ background: `linear-gradient(90deg, ${samples.join(", ")})` }}
    />
  );
}

export function Sidebar() {
  const devices = useDevices((s) => s.devices);
  const selectedId = useDevices((s) => s.selectedId);
  const select = useDevices((s) => s.select);
  const applyToAll = useDevices((s) => s.applyToAll);
  const [globalOpen, setGlobalOpen] = useState(false);

  return (
    <aside className="flex w-72 shrink-0 flex-col border-r border-panel-2 bg-panel">
      <div className="flex items-center gap-2 px-5 py-4">
        <div className="h-3 w-3 rounded-full bg-accent shadow-[0_0_10px_var(--color-accent)]" />
        <h1 className="text-lg font-bold tracking-wide">KontrolRGB</h1>
      </div>

      <nav className="flex-1 space-y-1 overflow-y-auto px-3 py-1">
        {devices.map((d) => (
          <button
            key={d.id}
            onClick={() => select(d.id)}
            className={`flex w-full items-center gap-3 rounded-lg px-3 py-3 text-left transition-colors ${
              d.id === selectedId
                ? "bg-panel-2 ring-1 ring-accent/50"
                : "hover:bg-panel-2/60"
            }`}
          >
            <span className="text-xl">{TYPE_ICONS[d.device_type]}</span>
            <span className="flex-1">
              <span className="block truncate text-sm font-medium">{d.name}</span>
              <span className="mt-1 block">
                <LiveSwatch device={d} />
              </span>
            </span>
          </button>
        ))}
      </nav>

      <div className="border-t border-panel-2 p-3">
        <button
          onClick={() => setGlobalOpen((o) => !o)}
          className="w-full rounded-lg bg-accent/90 px-3 py-2.5 text-sm font-semibold text-white transition-colors hover:bg-accent"
        >
          {globalOpen ? "Close" : "Apply to all devices"}
        </button>
        {globalOpen && (
          <div className="mt-3">
            <EffectPanel
              effects={["static", "breathing", "rainbow_wave", "color_cycle"]}
              onApply={(effect) => applyToAll(effect)}
            />
          </div>
        )}
      </div>
    </aside>
  );
}
