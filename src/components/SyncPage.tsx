import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { View } from "../App";
import { useDevices } from "../store/devices";
import { useSettings, type SyncEffectId } from "../store/settings";
import type { Color, DeviceInfo, DeviceType, EffectConfig } from "../types/device";
import {
  EFFECTS,
  EFFECT_BY_KIND,
  isEffectKind,
  capabilityFor,
  configForDevice,
} from "../lib/effectRegistry";
import type { EffectKind } from "../types/device";
import { ColorPickerPopover } from "./effects/ColorPickerPopover";

/* line-style device icons (inherit currentColor → tinted by --ac) */
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
    </svg>
  ),
};

const TYPE_ACCENT: Record<DeviceType, string> = {
  keyboard: "var(--kb)",
  motherboard: "var(--mb)",
  gpu: "var(--gpu)",
};

/* plain-English tile labels; the exact model lives in the tooltip. "Case Fans"
   because the board's ARGB header is what actually lights the fans. */
const TYPE_LABEL: Record<DeviceType, string> = {
  keyboard: "Keyboard",
  motherboard: "Case Fans",
  gpu: "GPU",
};

const CHECK = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round">
    <path d="M20 6 9 17l-5-5" />
  </svg>
);

/* One effect, every device. The browsable registry effects, each carrying the
   closest firmware (onboard) mode for devices that can't host-animate (GMMK). */
const SYNC_EFFECTS = EFFECTS.filter((e) => e.browsable);

const PRESETS: Color[] = [
  { r: 91, g: 140, b: 255 },  // blue
  { r: 34, g: 211, b: 238 },  // cyan
  { r: 168, g: 85, b: 247 },  // violet
  { r: 255, g: 90, b: 77 },   // red
  { r: 251, g: 191, b: 36 },  // amber
  { r: 132, g: 204, b: 22 },  // green
  { r: 255, g: 255, b: 255 }, // white
];

const sameColor = (a: Color, b: Color) => a.r === b.r && a.g === b.g && a.b === b.b;

function deviceSub(device: DeviceInfo): string {
  if (device.device_type === "keyboard") {
    const keys = device.zones.reduce((n, z) => n + z.led_count, 0);
    return `${keys} keys`;
  }
  const n = device.zones.length;
  return `${n} ${n === 1 ? "zone" : "zones"}`;
}

export function SyncPage({ onChangeView }: { onChangeView: (v: View) => void }) {
  const devices = useDevices((s) => s.devices);
  const states = useDevices((s) => s.states);
  const applyEffect = useDevices((s) => s.applyEffect);
  const applyBrightness = useDevices((s) => s.applyBrightness);

  // All sync controls live in the persisted settings store, so the landing page
  // comes back exactly as the user left it — across page changes, sessions, and
  // app restarts.
  const effectId = useSettings((s) => s.syncEffectId);
  const setEffectId = useSettings((s) => s.setSyncEffectId);
  const color = useSettings((s) => s.syncColor);
  const setColor = useSettings((s) => s.setSyncColor);
  const speed = useSettings((s) => s.syncSpeed);
  const setSpeed = useSettings((s) => s.setSyncSpeed);
  const reverse = useSettings((s) => s.syncReverse);
  const setReverse = useSettings((s) => s.setSyncReverse);
  // Closest firmware match vs. leave write-once devices out entirely — set on
  // the app-wide Settings page.
  const fallbackMode = useSettings((s) => s.syncFallbackMode);
  // Per-device include toggle. Missing → included (default on).
  const excluded = useSettings((s) => s.syncExcluded);
  const setExcluded = useSettings((s) => s.setSyncExcluded);

  // Fall back to the first sync effect if a stale/unknown id was persisted
  // (an older build, or an effect that later became non-browsable) — otherwise
  // dereferencing an undefined match below would white-screen the whole page.
  const effect = SYNC_EFFECTS.find((e) => e.kind === effectId) ?? SYNC_EFFECTS[0];

  // Effect picker dropdown — mirrors the per-device EffectPanel dropdown:
  // starred favorites first, then the rest, then a Browse Effects Library footer.
  const favorites = useSettings((s) => s.favoriteEffects);
  const syncKinds = SYNC_EFFECTS.map((e) => e.kind);
  const favs = favorites.filter(
    (k): k is EffectKind => isEffectKind(k) && syncKinds.includes(k),
  );
  const rest = syncKinds.filter((k) => !favs.includes(k));

  const [open, setOpen] = useState(false);
  const ddRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ddRef.current && !ddRef.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const isIncluded = (id: string) => !excluded[id];

  // Snapshot of each included device's effect at the moment "Turn all off" was
  // clicked, so the same button can restore it. null = lights on.
  const [offSnapshot, setOffSnapshot] = useState<Record<string, EffectConfig> | null>(null);

  // Per-device live state under the current effect + include/fallback choices.
  const plan = useMemo(
    () =>
      devices.map((d) => {
        const cap = capabilityFor(d.supported_effects, effect);
        const included = isIncluded(d.id);
        const skippedByFallback = cap === "fallback" && fallbackMode === "exclude";
        const live = included && cap !== "none" && !skippedByFallback;
        return { device: d, cap, included, live };
      }),
    [devices, effect, excluded, fallbackMode],
  );

  const liveDevices = plan.filter((p) => p.live);
  const n = liveDevices.length;

  // Live apply: any change to the effect or its settings — and opening the page
  // itself — pushes to every live device automatically (no Apply button). `plan`
  // covers effect/include/fallback/device-list changes, so this also fires once
  // the device list loads on first open.
  useEffect(() => {
    // Short debounce so slider drags don't spam the backend per pixel.
    const t = setTimeout(() => {
      for (const p of plan) {
        if (!p.live) continue;
        const cfg = configForDevice(p.device.supported_effects, effect, {
          color,
          speedPct: speed,
          reverse,
        });
        if (cfg) applyEffect(p.device.id, cfg);
      }
      // Any live apply relights the devices, so an earlier "Turn all off"
      // snapshot is stale — flip the button back to its off state.
      setOffSnapshot(null);
    }, 80);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, color, speed, reverse]);

  const turnAllOff = () => {
    const snap: Record<string, EffectConfig> = {};
    for (const p of plan) {
      if (!p.included) continue;
      const current = states[p.device.id]?.effect;
      if (current) snap[p.device.id] = current;
      applyEffect(p.device.id, { kind: "static", color: { r: 0, g: 0, b: 0 } });
    }
    setOffSnapshot(snap);
  };

  const turnAllOn = () => {
    if (!offSnapshot) return;
    for (const [id, effect] of Object.entries(offSnapshot)) applyEffect(id, effect);
    setOffSnapshot(null);
  };

  // Master brightness reflects the brightest included device; applied live.
  const masterBrightness =
    Math.max(0, ...liveDevices.map((p) => states[p.device.id]?.brightness ?? 100)) || 85;
  const setMaster = (b: number) => {
    for (const p of liveDevices) applyBrightness(p.device.id, b);
  };

  return (
    <main className="main">
      <div className="page sync-page">
        {/* header */}
        <div className="page-head">
          <div className="title">
            <div className="dev-ico sync-ico">
              <svg width="26" height="26" viewBox="0 0 32 32" aria-hidden="true">
                <circle cx="16" cy="5.5" r="2.4" fill="#ff5a4d" /><circle cx="25.1" cy="10.75" r="2.4" fill="#fbbf24" />
                <circle cx="25.1" cy="21.25" r="2.4" fill="#84cc16" /><circle cx="16" cy="26.5" r="2.4" fill="#22d3ee" />
                <circle cx="6.9" cy="21.25" r="2.4" fill="#5b8cff" /><circle cx="6.9" cy="10.75" r="2.4" fill="#a855f7" />
                <circle cx="16" cy="16" r="2.4" fill="#e7e9ee" />
              </svg>
            </div>
            <div>
              <h2>Sync All Devices</h2>
              <div className="sub">
                <span className="ok">● {devices.length} lighting connected</span>
                <span>One effect, every device</span>
              </div>
            </div>
          </div>
          <div className="row" style={{ gap: 18 }}>
            <div style={{ textAlign: "right" }}>
              <div className="muted" style={{ marginBottom: 6 }}>Master brightness</div>
              <input
                className="slider"
                type="range"
                min={0}
                max={100}
                value={masterBrightness}
                style={{ width: 160, ["--p" as string]: `${masterBrightness}%` }}
                onChange={(e) => setMaster(Number(e.target.value))}
              />
            </div>
            <button
              className="btn ghost"
              onClick={offSnapshot ? turnAllOn : turnAllOff}
              title={
                offSnapshot
                  ? "Restore each device's previous effect"
                  : "Turn off every included device"
              }
            >
              {offSnapshot ? "Turn all on" : "Turn all off"}
            </button>
          </div>
        </div>

        {/* one calm preview strip */}
        <div className="preview">
          <div className={`bar ${effect.pv}`} style={{ ["--pick" as string]: cssHex(color) }} />
          <div className="cap">
            <span className="l"><span className="live" /> Preview · <b>{effect.label}</b></span>
            <span className="r">on {n} device{n === 1 ? "" : "s"}</span>
          </div>
        </div>

        {/* device tiles : click to include / exclude */}
        <div className="dev-tiles">
          {plan.map(({ device, cap, included }) => {
            const off = !included || cap === "none";
            const badge =
              included && cap === "fallback" ? `≈ ${effect.label}` : null;
            return (
              <button
                key={device.id}
                className={`tile-dev ${off ? "off" : "on"}`}
                style={{ ["--ac" as string]: TYPE_ACCENT[device.device_type] }}
                onClick={() => setExcluded({ ...excluded, [device.id]: included })}
                disabled={cap === "none"}
                title={cap === "none" ? `${device.name} can't run the selected effect` : device.name}
              >
                <span className="check">{CHECK}</span>
                <span className="ico">{TYPE_ICONS[device.device_type]}</span>
                <span className="meta">
                  <span className="nm">{TYPE_LABEL[device.device_type]}</span>
                  <span className="sub">{deviceSub(device)}</span>
                </span>
                {badge && <span className="badge">{badge}</span>}
              </button>
            );
          })}
        </div>

        {/* effect + settings */}
        <div className="grid-2 sync-grid">
          <div className="card">
            <div className="card-head">
              <h3>Pick one effect</h3>
              <span className="chip"><span className="led" style={{ background: "var(--seg)" }} /> {effect.label}</span>
            </div>
            <div className="card-pad">
              <div className="fx-dd" ref={ddRef}>
                <button
                  className={`fx-dd-btn ${open ? "open" : ""}`}
                  onClick={() => setOpen((o) => !o)}
                  aria-haspopup="listbox"
                  aria-expanded={open}
                >
                  <span className={`pv ${effect.pv}`} />
                  <span className="meta">
                    <span className="nm">{effect.label}</span>
                    <span className="ds">{effect.description}</span>
                  </span>
                  <svg className="chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <path d="m6 9 6 6 6-6" />
                  </svg>
                </button>

                {open && (
                  <div className="fx-dd-menu" role="listbox">
                    {[...favs, ...rest].map((kind) => (
                      <button
                        key={kind}
                        role="option"
                        aria-selected={kind === effectId}
                        className={`fx-dd-item ${kind === effectId ? "on" : ""}`}
                        onClick={() => { setEffectId(kind as SyncEffectId); setOpen(false); }}
                      >
                        <span className={`pv ${EFFECT_BY_KIND[kind].pv}`} />
                        <span className="nm">{EFFECT_BY_KIND[kind].label}</span>
                        {favs.includes(kind) && (
                          <svg className="fav" viewBox="0 0 24 24" fill="currentColor">
                            <path d="M12 2.5 14.9 8.6l6.6.8-4.9 4.5 1.3 6.5-5.9-3.3-5.9 3.3 1.3-6.5L2.5 9.4l6.6-.8z" />
                          </svg>
                        )}
                        {kind === effectId && (
                          <svg className="chk" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
                            <path d="m5 12.5 4.5 4.5L19 7.5" />
                          </svg>
                        )}
                      </button>
                    ))}

                    <div className="fx-dd-sep" />
                    <button
                      className="fx-dd-item browse"
                      onClick={() => { setOpen(false); onChangeView("library"); }}
                    >
                      <span className="pv pv-dd-browse">
                        <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                          <circle cx="11" cy="11" r="7" />
                          <path d="m20 20-3.5-3.5" />
                        </svg>
                      </span>
                      <span className="nm">Browse Effects Library…</span>
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>

          <div className="card">
            <div className="card-head"><h3>Settings</h3></div>
            <div className="card-pad">
              <div className={`color-block ${effect.supports.color ? "" : "dim"}`}>
                <div className="muted" style={{ marginBottom: 9 }}>Color</div>
                <div className="swatch-row" style={{ marginBottom: 14 }}>
                  {PRESETS.map((c, i) => (
                    <button
                      key={i}
                      className={`col ${sameColor(c, color) ? "sel" : ""}`}
                      style={{ background: cssHex(c) }}
                      onClick={() => setColor(c)}
                    />
                  ))}
                  <ColorPickerPopover color={color} onChange={setColor} align="right" />
                </div>
              </div>
              <div style={{ borderTop: "1px solid var(--line)", paddingTop: 6 }}>
                {effect.supports.speed && (
                  <div className="ctrl-row">
                    <span className="lbl">Speed</span>
                    <input
                      className="slider"
                      type="range"
                      min={0}
                      max={100}
                      value={speed}
                      style={{ ["--p" as string]: `${speed}%` }}
                      onChange={(e) => setSpeed(Number(e.target.value))}
                    />
                  </div>
                )}
                {effect.supports.direction && (
                  <div className="ctrl-row">
                    <span className="lbl">Direction</span>
                    <div className="seg-ctrl">
                      <button className={!reverse ? "on" : ""} onClick={() => setReverse(false)}>→ Forward</button>
                      <button className={reverse ? "on" : ""} onClick={() => setReverse(true)}>← Reverse</button>
                    </div>
                  </div>
                )}
                {!effect.supports.speed && !effect.supports.direction && (
                  <p className="muted" style={{ margin: "4px 0 0" }}>
                    A solid color held on every device. Pick a hue above.
                  </p>
                )}
              </div>
            </div>
          </div>
        </div>

        <button className="sync-jump" onClick={() => onChangeView("devices")}>
          Need finer control? Open a single device →
        </button>
      </div>
    </main>
  );
}

const hex2 = (n: number) => n.toString(16).padStart(2, "0");
const cssHex = (c: Color) => `#${hex2(c.r)}${hex2(c.g)}${hex2(c.b)}`;
