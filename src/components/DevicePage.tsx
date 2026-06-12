import { useDevices } from "../store/devices";
import { BrightnessSlider } from "./effects/BrightnessSlider";
import { ColorPickerPopover } from "./effects/ColorPickerPopover";
import { EffectPanel } from "./effects/EffectPanel";
import { ZoneView } from "./zones/ZoneView";

export function DevicePage() {
  const device = useDevices((s) =>
    s.devices.find((d) => d.id === s.selectedId),
  );
  const state = useDevices((s) =>
    s.selectedId ? s.states[s.selectedId] : undefined,
  );
  const applyEffect = useDevices((s) => s.applyEffect);
  const applyBrightness = useDevices((s) => s.applyBrightness);
  const paintColor = useDevices((s) => s.paintColor);
  const setPaintColor = useDevices((s) => s.setPaintColor);
  const rescan = useDevices((s) => s.rescan);

  if (!device) {
    return (
      <main className="flex flex-1 items-center justify-center text-zinc-500">
        No device selected
      </main>
    );
  }

  return (
    <main className="flex-1 overflow-y-auto">
      <div className="mx-auto max-w-5xl space-y-6 p-6">
        <header className="flex items-center justify-between gap-4">
          <h2 className="text-xl font-bold">{device.name}</h2>
          <div className="flex items-center gap-4">
            {state && (
              <BrightnessSlider
                value={state.brightness}
                onChange={(b) => applyBrightness(device.id, b)}
              />
            )}
            <button
              onClick={rescan}
              className="rounded-lg bg-panel-2 px-3 py-1.5 text-sm text-zinc-300 hover:bg-zinc-700"
              title="Re-detect devices"
            >
              ⟳ Rescan
            </button>
          </div>
        </header>

        <section className="space-y-5">
          {device.zones.map((zone) => (
            <ZoneView key={zone.id} deviceId={device.id} zone={zone} />
          ))}
        </section>

        <section className="rounded-xl border border-panel-2 bg-panel p-5">
          <div className="mb-4 flex items-center justify-between">
            <h3 className="text-sm font-semibold uppercase tracking-wider text-zinc-400">
              Effect
            </h3>
            <div className="flex items-center gap-2 text-xs text-zinc-400">
              Paint color
              <ColorPickerPopover
                color={paintColor}
                onChange={setPaintColor}
                align="right"
              />
            </div>
          </div>
          {state && (
            <EffectPanel
              effects={device.supported_effects}
              value={state.effect}
              onApply={(effect) => applyEffect(device.id, effect)}
            />
          )}
        </section>
      </div>
    </main>
  );
}
