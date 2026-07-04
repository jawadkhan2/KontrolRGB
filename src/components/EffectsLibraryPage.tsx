import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import { useDevices } from "../store/devices";
import { useSettings } from "../store/settings";
import type { Color, DeviceType, EffectKind } from "../types/device";
import {
  EFFECTS,
  EFFECT_CATEGORIES,
  type EffectCategory,
  type EffectMeta,
  capabilityFor,
  configForDevice,
  matchesQuery,
} from "../lib/effectRegistry";
import { ColorPickerPopover } from "./effects/ColorPickerPopover";

/** "all" or a device id. */
export type LibraryTarget = string;

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

const StarIcon = ({ filled }: { filled: boolean }) => (
  <svg viewBox="0 0 24 24" fill={filled ? "currentColor" : "none"} stroke="currentColor" strokeWidth="1.8" strokeLinejoin="round">
    <path d="m12 3 2.7 5.8 6.3.7-4.7 4.3 1.3 6.2L12 16.9 6.4 20l1.3-6.2L3 9.5l6.3-.7Z" />
  </svg>
);

const PRESETS: Color[] = [
  { r: 91, g: 140, b: 255 },  // blue
  { r: 34, g: 211, b: 238 },  // cyan
  { r: 168, g: 85, b: 247 },  // violet
  { r: 255, g: 90, b: 77 },   // red
  { r: 251, g: 191, b: 36 },  // amber
  { r: 132, g: 204, b: 22 },  // green
  { r: 255, g: 255, b: 255 }, // white
];

const hex2 = (n: number) => n.toString(16).padStart(2, "0");
const cssHex = (c: Color) => `#${hex2(c.r)}${hex2(c.g)}${hex2(c.b)}`;
const sameColor = (a: Color, b: Color) => a.r === b.r && a.g === b.g && a.b === b.b;

const LIBRARY = EFFECTS.filter((e) => e.browsable);

export function EffectsLibraryPage({
  target,
  onChangeTarget,
}: {
  target: LibraryTarget;
  onChangeTarget: (t: LibraryTarget) => void;
}) {
  const devices = useDevices((s) => s.devices);
  const applyEffect = useDevices((s) => s.applyEffect);
  const favorites = useSettings((s) => s.favoriteEffects);
  const toggleFavorite = useSettings((s) => s.toggleFavoriteEffect);

  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<EffectCategory | "All">("All");
  // Nothing applies until the user picks a card — browsing is passive.
  const [selected, setSelected] = useState<EffectKind | null>(null);
  const [color, setColor] = useState<Color>(PRESETS[0]);
  const [speedPct, setSpeedPct] = useState(55);
  const [reverse, setReverse] = useState(false);

  const targetDevice = devices.find((d) => d.id === target) ?? null;
  const targetDevices = targetDevice ? [targetDevice] : devices;

  /** Best capability across the target set — drives card badges/dimming. */
  const capOf = (meta: EffectMeta) => {
    let best: "none" | "fallback" | "native" = "none";
    for (const d of targetDevices) {
      const c = capabilityFor(d.supported_effects, meta);
      if (c === "native") return "native";
      if (c === "fallback") best = "fallback";
    }
    return best;
  };

  const shown = useMemo(() => {
    const list = LIBRARY.filter(
      (e) =>
        (category === "All" || e.category === category) &&
        matchesQuery(e, query),
    );
    // Favorites first (in starred order), rest keep registry order.
    return [...list].sort((a, b) => {
      const fa = favorites.indexOf(a.kind);
      const fb = favorites.indexOf(b.kind);
      if (fa !== -1 || fb !== -1) {
        if (fa === -1) return 1;
        if (fb === -1) return -1;
        return fa - fb;
      }
      return 0;
    });
  }, [category, query, favorites]);

  const selectedMeta = selected
    ? LIBRARY.find((e) => e.kind === selected) ?? null
    : null;

  const push = (meta: EffectMeta, opts: { color: Color; speedPct: number; reverse: boolean }) => {
    for (const d of targetDevices) {
      const cfg = configForDevice(d.supported_effects, meta, opts);
      if (cfg) applyEffect(d.id, cfg);
    }
  };

  const pick = (meta: EffectMeta) => {
    setSelected(meta.kind);
    push(meta, { color, speedPct, reverse });
  };

  // Live re-apply on control changes (debounced so slider drags don't spam the
  // backend). The run triggered by pick() itself is skipped — pick() already
  // applied the effect.
  const lastPicked = useRef<EffectKind | null>(null);
  useEffect(() => {
    if (!selectedMeta) return;
    if (lastPicked.current !== selectedMeta.kind) {
      lastPicked.current = selectedMeta.kind;
      return;
    }
    const t = setTimeout(() => push(selectedMeta, { color, speedPct, reverse }), 80);
    return () => clearTimeout(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedMeta, color, speedPct, reverse]);

  const n = targetDevices.length;

  return (
    <main className="main">
      <div className="page">
        {/* header */}
        <div className="page-head">
          <div className="title">
            <div className="dev-ico">
              <svg viewBox="0 0 24 24" width="24" height="24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
                <rect x="3" y="3" width="7.5" height="7.5" rx="1.6" />
                <rect x="13.5" y="3" width="7.5" height="7.5" rx="1.6" />
                <rect x="3" y="13.5" width="7.5" height="7.5" rx="1.6" />
                <path d="M17.25 13.5v7.5M13.5 17.25H21" />
              </svg>
            </div>
            <div>
              <h2>Effects Library</h2>
              <div className="sub">
                <span>{LIBRARY.length} effects</span>
                <span>Pick one to apply it live</span>
              </div>
            </div>
          </div>
        </div>

        {/* search + category filter */}
        <div className="lib-toolbar">
          <div className="lib-search">
            <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
              <circle cx="11" cy="11" r="7" />
              <path d="m20 20-3.5-3.5" />
            </svg>
            <input
              type="text"
              placeholder="Search effects… (wave, sparkle, scanner)"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
            {query && (
              <button className="clear" onClick={() => setQuery("")} aria-label="Clear search">✕</button>
            )}
          </div>
          <div className="cat-chips">
            {(["All", ...EFFECT_CATEGORIES] as const).map((c) => (
              <button key={c} className={category === c ? "on" : ""} onClick={() => setCategory(c)}>
                {c}
              </button>
            ))}
          </div>
        </div>

        {/* apply target */}
        <div className="lib-target">
          <span className="lbl">Apply to</span>
          <div className="cat-chips">
            <button className={!targetDevice ? "on" : ""} onClick={() => onChangeTarget("all")}>
              All devices
            </button>
            {devices.map((d) => (
              <button
                key={d.id}
                className={`with-ico ${target === d.id ? "on" : ""}`}
                onClick={() => onChangeTarget(d.id)}
              >
                <span className="ico">{TYPE_ICONS[d.device_type]}</span>
                {d.name}
              </button>
            ))}
          </div>
        </div>

        <div className="lib-body">
          {/* card grid */}
          <div className="lib-grid">
            {shown.map((e) => {
              const cap = capOf(e);
              const fav = favorites.includes(e.kind);
              return (
                <button
                  key={e.kind}
                  className={`fx-card ${selected === e.kind ? "on" : ""} ${cap === "none" ? "off" : ""}`}
                  disabled={cap === "none"}
                  onClick={() => pick(e)}
                  title={cap === "none" ? "The selected target can't run this effect" : undefined}
                >
                  <div className={`pv ${e.pv}`} style={{ ["--pick" as string]: cssHex(color) }} />
                  <span
                    className={`star ${fav ? "fav" : ""}`}
                    role="button"
                    tabIndex={0}
                    aria-label={fav ? "Unstar effect" : "Star effect"}
                    onClick={(ev) => { ev.stopPropagation(); toggleFavorite(e.kind); }}
                    onKeyDown={(ev) => {
                      if (ev.key === "Enter" || ev.key === " ") {
                        ev.preventDefault();
                        ev.stopPropagation();
                        toggleFavorite(e.kind);
                      }
                    }}
                  >
                    <StarIcon filled={fav} />
                  </span>
                  <div className="nm">{e.label}</div>
                  <div className="ds">{e.description}</div>
                  <div className="ft">
                    <span className="cat">{e.category}</span>
                    {cap === "fallback" && <span className="badge">≈ firmware</span>}
                  </div>
                </button>
              );
            })}
            {shown.length === 0 && (
              <div className="lib-empty">
                No effects match “{query}”. Try “wave”, “sparkle”, or clear the search.
              </div>
            )}
          </div>

          {/* sticky controls */}
          <div className="card lib-controls">
            <div className="card-head">
              <h3>Settings</h3>
              {selectedMeta && (
                <span className="chip">
                  <span className="led" style={{ background: "var(--seg)" }} /> {selectedMeta.label}
                </span>
              )}
            </div>
            <div className="card-pad">
              {!selectedMeta ? (
                <p className="muted" style={{ margin: 0 }}>
                  Pick an effect to apply it to <b style={{ color: "var(--text)" }}>
                  {targetDevice ? targetDevice.name : `all ${n} devices`}</b> and tune it here.
                </p>
              ) : (
                <>
                  <div className={`color-block ${selectedMeta.supports.color ? "" : "dim"}`}>
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
                    {selectedMeta.supports.speed && (
                      <div className="ctrl-row">
                        <span className="lbl">Speed</span>
                        <input
                          className="slider"
                          type="range"
                          min={0}
                          max={100}
                          value={speedPct}
                          style={{ ["--p" as string]: `${speedPct}%` }}
                          onChange={(e) => setSpeedPct(Number(e.target.value))}
                        />
                      </div>
                    )}
                    {selectedMeta.supports.direction && (
                      <div className="ctrl-row">
                        <span className="lbl">Direction</span>
                        <div className="seg-ctrl">
                          <button className={!reverse ? "on" : ""} onClick={() => setReverse(false)}>→ Forward</button>
                          <button className={reverse ? "on" : ""} onClick={() => setReverse(true)}>← Reverse</button>
                        </div>
                      </div>
                    )}
                    {!selectedMeta.supports.speed && !selectedMeta.supports.direction && (
                      <p className="muted" style={{ margin: "4px 0 0" }}>
                        A solid color held on the target. Pick a hue above.
                      </p>
                    )}
                  </div>
                  <p className="muted" style={{ margin: "10px 0 0" }}>
                    Applied live to <b style={{ color: "var(--text)" }}>
                    {targetDevice ? targetDevice.name : `${n} devices`}</b>.
                  </p>
                </>
              )}
            </div>
          </div>
        </div>
      </div>
    </main>
  );
}
