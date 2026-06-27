import type { ReactNode } from "react";
import { useDevices } from "../store/devices";
import type { Color, DeviceType, EffectKind } from "../types/device";
import { BrightnessSlider } from "./effects/BrightnessSlider";
import { ColorPickerPopover } from "./effects/ColorPickerPopover";
import { EffectPanel } from "./effects/EffectPanel";
import { ZoneView } from "./zones/ZoneView";
import { ArgbHeaderStrip } from "./zones/ArgbHeaderStrip";
import { MotherboardBoard } from "./zones/MotherboardBoard";
import { GpuRender } from "./zones/GpuRender";

const TYPE_ICONS: Record<DeviceType, ReactNode> = {
  keyboard: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round">
      <rect x="2" y="6" width="20" height="12" rx="2.5" />
      <path d="M6 9.5h0M9.5 9.5h0M13 9.5h0M16.5 9.5h0M6 13h0M16.5 13h0" strokeWidth="2.2" />
      <path d="M9 13h6" />
    </svg>
  ),
  motherboard: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3.5" y="3.5" width="17" height="17" rx="2" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
      <path d="M9 3.5v-1M15 3.5v-1M9 21.5v1M15 21.5v1M3.5 9h-1M3.5 15h-1M21.5 9h1M21.5 15h1" />
    </svg>
  ),
  gpu: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <circle cx="8.5" cy="12" r="2.6" />
      <circle cx="15.5" cy="12" r="2.6" />
      <path d="M8.5 12h0M15.5 12h0" strokeWidth="2" />
    </svg>
  ),
};

const EFFECT_LABELS: Record<EffectKind, string> = {
  static: "Static",
  breathing: "Breathing",
  rainbow_wave: "Rainbow Wave",
  color_cycle: "Color Cycle",
  custom: "Custom",
  onboard: "Onboard",
};

/** Quick-pick paint colors (mirrors the redesign swatch row). */
const PRESETS: Color[] = [
  { r: 34, g: 211, b: 238 },  // cyan
  { r: 91, g: 140, b: 255 },  // blue
  { r: 168, g: 85, b: 247 },  // violet
  { r: 255, g: 90, b: 77 },   // red
  { r: 251, g: 191, b: 36 },  // amber
  { r: 132, g: 204, b: 22 },  // green
  { r: 255, g: 255, b: 255 }, // white
];

/** GPU telemetry tiles (values pending the RTX driver backend). */
const GPU_TELEM = [
  { label: "GPU Temp", unit: "°C" },
  { label: "Core Clock", unit: "MHz" },
  { label: "Fan Speed", unit: "%" },
  { label: "Power", unit: "W" },
];

const hex2 = (n: number) => n.toString(16).padStart(2, "0");
const toHex = (c: Color) => `#${hex2(c.r)}${hex2(c.g)}${hex2(c.b)}`.toUpperCase();
const sameColor = (a: Color, b: Color) => a.r === b.r && a.g === b.g && a.b === b.b;

export function DevicePage() {
  const device = useDevices((s) => s.devices.find((d) => d.id === s.selectedId));
  const state = useDevices((s) => (s.selectedId ? s.states[s.selectedId] : undefined));
  const applyEffect = useDevices((s) => s.applyEffect);
  const applyBrightness = useDevices((s) => s.applyBrightness);
  const paintColor = useDevices((s) => s.paintColor);
  const setPaintColor = useDevices((s) => s.setPaintColor);
  const identifyZone = useDevices((s) => s.identifyZone);
  const rescan = useDevices((s) => s.rescan);

  if (!device) {
    return <main className="main flex items-center justify-center text-faint">No device selected</main>;
  }

  const ledTotal = device.zones.reduce((n, z) => n + z.led_count, 0);
  const isKeyboard = device.device_type === "keyboard";
  const isMotherboard = device.device_type === "motherboard";
  const isGpu = device.device_type === "gpu";
  const zoneWord = device.zones.length === 1 ? "zone" : "zones";

  const effectCard = state && (
    <div className="card">
      <div className="card-head">
        <h3>Effect</h3>
        <span className="chip"><span className="led" style={{ background: "var(--seg)" }} /> {EFFECT_LABELS[state.effect.kind]}</span>
      </div>
      <div className="card-pad">
        <EffectPanel
          key={device.id}
          effects={device.supported_effects}
          value={state.effect}
          onApply={(effect) => applyEffect(device.id, effect)}
        />
      </div>
    </div>
  );

  const paintCard = (
    <div className="card">
      <div className="card-head"><h3>{isGpu ? "Color" : "Paint Color"}</h3></div>
      <div className="card-pad">
        <div className="swatch-row" style={{ marginBottom: 14 }}>
          {PRESETS.map((c, i) => (
            <button
              key={i}
              className={`col ${sameColor(c, paintColor) ? "sel" : ""}`}
              style={{ background: `rgb(${c.r},${c.g},${c.b})` }}
              onClick={() => setPaintColor(c)}
              title={toHex(c)}
            />
          ))}
          <ColorPickerPopover color={paintColor} onChange={setPaintColor} align="right" />
        </div>
        <div className="color-bar" style={{ background: `linear-gradient(90deg,#000,${toHex(paintColor)})` }} />
        <div className="muted" style={{ marginTop: 8 }}>
          Hex <b style={{ color: "var(--text)" }}>{toHex(paintColor)}</b> · R{paintColor.r} G{paintColor.g} B{paintColor.b}
        </div>
        {isGpu && (
          <p className="muted" style={{ margin: "12px 0 0" }}>
            <b style={{ color: "var(--text)" }}>Temp Color</b> maps the halo from cyan (cool) → red (hot) as the die
            heats up — RGB that actually tells you something.
          </p>
        )}
      </div>
    </div>
  );

  // GPU mirrors the approved mockup: a telemetry bar, then the WindForce render
  // beside the Effect/Color stack (rather than a full-width hero).
  if (isGpu) {
    return (
      <main className="main">
        <div className="page">
          <div className="page-head">
            <div className="title">
              <div className="dev-ico">{TYPE_ICONS[device.device_type]}</div>
              <div>
                <h2>{device.name}</h2>
                <div className="sub">
                  <span className="ok">● Connected</span>
                  <span>{device.zones.length} RGB {zoneWord}</span>
                  <span style={{ textTransform: "capitalize" }}>{device.device_type}</span>
                </div>
              </div>
            </div>
            <div className="row" style={{ gap: 18 }}>
              <div style={{ textAlign: "right" }}>
                <div className="muted" style={{ marginBottom: 6 }}>Brightness</div>
                {state && <BrightnessSlider value={state.brightness} onChange={(b) => applyBrightness(device.id, b)} />}
              </div>
              <button className="btn ghost" onClick={rescan} title="Re-detect devices">⟳</button>
            </div>
          </div>

          {/* telemetry — populated once the RTX driver backend lands */}
          <div className="card" style={{ marginBottom: 16 }}>
            <div className="card-pad">
              <div className="gpu-telem">
                {GPU_TELEM.map((t) => (
                  <div key={t.label}>
                    <div className="muted">{t.label}</div>
                    <div className="gv">—<small> {t.unit}</small></div>
                    <div className="gbar"><span style={{ width: 0 }} /></div>
                  </div>
                ))}
              </div>
              <p className="muted" style={{ margin: "12px 0 0" }}>
                Live telemetry (temp · clock · fan · power) arrives with the RTX driver backend.
              </p>
            </div>
          </div>

          <div className="grid-2">
            <GpuRender deviceId={device.id} device={device} />
            <div className="stack">
              {effectCard}
              {paintCard}
            </div>
          </div>
        </div>
      </main>
    );
  }

  return (
    <main className="main">
      <div className="page">
        {/* header */}
        <div className="page-head">
          <div className="title">
            <div className="dev-ico">{TYPE_ICONS[device.device_type]}</div>
            <div>
              <h2>{device.name}</h2>
              <div className="sub">
                <span className="ok">● Connected</span>
                <span>{ledTotal} {isKeyboard ? "keys" : "LEDs"} · {device.zones.length} {zoneWord}</span>
                <span style={{ textTransform: "capitalize" }}>{device.device_type}</span>
              </div>
            </div>
          </div>
          <div className="row" style={{ gap: 18 }}>
            <div style={{ textAlign: "right" }}>
              <div className="muted" style={{ marginBottom: 6 }}>Brightness</div>
              {state && <BrightnessSlider value={state.brightness} onChange={(b) => applyBrightness(device.id, b)} />}
            </div>
            <button className="btn ghost" onClick={rescan} title="Re-detect devices">⟳</button>
          </div>
        </div>

        {/* live hardware stage */}
        {isMotherboard ? (
          <MotherboardBoard deviceId={device.id} device={device} />
        ) : (
          <div className="dev-stage">
            {device.zones.map((zone) => (
              <ZoneView key={zone.id} deviceId={device.id} zone={zone} />
            ))}
            <div className="stage-legend">
              <div className="item">
                <span className="sw" style={{ background: "var(--seg)" }} />
                {state ? EFFECT_LABELS[state.effect.kind] : "—"} · live preview
              </div>
              <div className="item">
                <span className="sw" style={{ background: "linear-gradient(90deg,#ff5a4d,#fbbf24,#84cc16,#22d3ee,#5b8cff)" }} />
                Switch to Custom to paint individual {isKeyboard ? "keys" : "LEDs"}
              </div>
            </div>
          </div>
        )}

        {/* controls */}
        <div className="grid-2">
          {effectCard}

          <div className="stack">
            {paintCard}

            {!isMotherboard && (
              <div className="card">
                <div className="card-head"><h3>Zone{device.zones.length > 1 ? "s" : ` · ${device.zones[0]?.name ?? ""}`}</h3><span className="muted">{ledTotal} LEDs</span></div>
                <div className="card-pad">
                  <div className="row spread">
                    <button className="btn" style={{ flex: 1 }} onClick={() => device.zones[0] && identifyZone(device.id, device.zones[0].id)}>◉ Pulse to identify</button>
                    <button className="btn" style={{ flex: 1 }} onClick={() => applyEffect(device.id, { kind: "static", color: { r: 0, g: 0, b: 0 } })}>Clear</button>
                  </div>
                  <p className="muted" style={{ margin: "12px 0 0" }}>
                    {isKeyboard
                      ? <>The keyboard exposes its keys as one paintable zone. Switch to <b style={{ color: "var(--text)" }}>Custom</b> to click individual keys above.</>
                      : <>Switch to <b style={{ color: "var(--text)" }}>Custom</b> to paint individual LEDs above.</>}
                  </p>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* paintable ARGB header strips (board render above is decorative) */}
        {isMotherboard && (
          <div className="card" style={{ marginTop: 16 }}>
            <div className="card-head"><h3>ARGB Headers</h3><span className="muted">Addressable strips · pick Custom to paint</span></div>
            <div className="card-pad">
              {device.zones.map((z) => (
                <ArgbHeaderStrip key={z.id} deviceId={device.id} zone={z} />
              ))}
            </div>
          </div>
        )}
      </div>
    </main>
  );
}
