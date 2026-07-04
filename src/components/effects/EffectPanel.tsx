import { useEffect, useRef, useState } from "react";
import type {
  EffectConfig,
  EffectKind,
  OnboardMode,
} from "../../types/device";
import {
  EFFECT_BY_KIND,
  defaultEffect,
  isEffectKind,
} from "../../lib/effectRegistry";
import { useSettings } from "../../store/settings";
import { ColorPickerPopover } from "./ColorPickerPopover";

const ONBOARD_MODE_LABELS: Record<OnboardMode, string> = {
  fixed: "Fixed",
  breathing: "Breathing",
  wave: "Wave",
  spectrum: "Spectrum",
  reactive: "Reactive",
  swirl: "Swirl",
};

/** Modes whose base color is ignored (firmware drives its own hue). */
const ONBOARD_COLORLESS: OnboardMode[] = ["spectrum"];
/** Modes that support a direction toggle. */
const ONBOARD_DIRECTIONAL: OnboardMode[] = ["wave", "swirl"];
/** Modes with no animation, so a speed control is meaningless. */
const ONBOARD_SPEEDLESS: OnboardMode[] = ["fixed"];

interface Props {
  effects: string[];
  /** Current effect (per-device mode). Omit for global mode. */
  value?: EffectConfig;
  onApply: (effect: EffectConfig) => void;
  /** When set, the dropdown gets a "Browse Effects Library" footer entry. */
  onBrowseLibrary?: () => void;
}

export function EffectPanel({ effects, value, onApply, onBrowseLibrary }: Props) {
  const [effect, setEffect] = useState<EffectConfig>(
    value ?? { kind: "rainbow_wave", speed: 1.0, reverse: false },
  );
  const favorites = useSettings((s) => s.favoriteEffects);
  const [open, setOpen] = useState(false);
  const ddRef = useRef<HTMLDivElement>(null);

  // Close the dropdown on outside click / Escape.
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
  // Remembers the last-used config per effect kind, so switching away and back
  // within a session restores its settings instead of resetting to defaults.
  const memory = useRef<Partial<Record<EffectKind, EffectConfig>>>({});
  const applyTimer = useRef<number | null>(null);
  const pendingEffect = useRef<EffectConfig | null>(null);

  useEffect(() => {
    if (value) {
      setEffect(value);
      memory.current[value.kind] = value;
    }
  }, [value]);

  useEffect(
    () => () => {
      if (applyTimer.current !== null) window.clearTimeout(applyTimer.current);
    },
    [],
  );

  const applyNow = (next: EffectConfig) => {
    if (applyTimer.current !== null) window.clearTimeout(applyTimer.current);
    applyTimer.current = null;
    pendingEffect.current = null;
    onApply(next);
  };

  const update = (next: EffectConfig, immediate = true) => {
    memory.current[next.kind] = next;
    setEffect(next);
    if (immediate) {
      applyNow(next);
      return;
    }
    pendingEffect.current = next;
    if (applyTimer.current !== null) window.clearTimeout(applyTimer.current);
    applyTimer.current = window.setTimeout(() => applyNow(next), 120);
  };

  const flushPending = () => {
    if (pendingEffect.current) applyNow(pendingEffect.current);
  };

  /** Switch to an effect kind, restoring its remembered settings if any. */
  const selectKind = (kind: EffectKind) => {
    update(memory.current[kind] ?? defaultEffect(kind, effect));
  };

  const supported = effects.filter(isEffectKind);

  // Dropdown ordering: starred favorites first (in the order they were
  // starred), then the remaining host effects, then the device-only modes
  // (Custom / Onboard) behind a divider.
  const favs = favorites.filter(
    (k): k is EffectKind => isEffectKind(k) && supported.includes(k),
  );
  const rest = supported.filter((k) => EFFECT_BY_KIND[k].browsable && !favs.includes(k));
  const specials = supported.filter((k) => !EFFECT_BY_KIND[k].browsable);

  // Which controls apply to the current effect.
  const meta = EFFECT_BY_KIND[effect.kind];
  const onboardMode = effect.kind === "onboard" ? effect.mode : null;
  const showColor =
    meta.supports.color ||
    (effect.kind === "onboard" && !effect.rainbow && !ONBOARD_COLORLESS.includes(effect.mode));
  const showSpeed =
    meta.supports.speed ||
    (effect.kind === "onboard" && !ONBOARD_SPEEDLESS.includes(effect.mode));
  const showDirection =
    meta.supports.direction ||
    (effect.kind === "onboard" && ONBOARD_DIRECTIONAL.includes(effect.mode));
  const showRainbow =
    effect.kind === "onboard" && !ONBOARD_COLORLESS.includes(effect.mode);
  const reverse = "reverse" in effect ? effect.reverse : false;
  const onboardSpeed = effect.kind === "onboard";

  return (
    <div>
      {/* effect selector */}
      <div className="fx-dd" ref={ddRef}>
        <button
          className={`fx-dd-btn ${open ? "open" : ""}`}
          onClick={() => setOpen((o) => !o)}
          aria-haspopup="listbox"
          aria-expanded={open}
        >
          <span className={`pv ${meta.pv}`} />
          <span className="meta">
            <span className="nm">{meta.label}</span>
            <span className="ds">{meta.description}</span>
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
                aria-selected={effect.kind === kind}
                className={`fx-dd-item ${effect.kind === kind ? "on" : ""}`}
                onClick={() => { selectKind(kind); setOpen(false); }}
              >
                <span className={`pv ${EFFECT_BY_KIND[kind].pv}`} />
                <span className="nm">{EFFECT_BY_KIND[kind].label}</span>
                {favs.includes(kind) && (
                  <svg className="fav" viewBox="0 0 24 24" fill="currentColor">
                    <path d="M12 2.5 14.9 8.6l6.6.8-4.9 4.5 1.3 6.5-5.9-3.3-5.9 3.3 1.3-6.5L2.5 9.4l6.6-.8z" />
                  </svg>
                )}
                {effect.kind === kind && (
                  <svg className="chk" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
                    <path d="m5 12.5 4.5 4.5L19 7.5" />
                  </svg>
                )}
              </button>
            ))}

            {specials.length > 0 && <div className="fx-dd-sep" />}
            {specials.map((kind) => (
              <button
                key={kind}
                role="option"
                aria-selected={effect.kind === kind}
                className={`fx-dd-item ${effect.kind === kind ? "on" : ""}`}
                onClick={() => { selectKind(kind); setOpen(false); }}
              >
                <span className={`pv ${EFFECT_BY_KIND[kind].pv}`} />
                <span className="nm">{EFFECT_BY_KIND[kind].label}</span>
                {effect.kind === kind && (
                  <svg className="chk" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.4" strokeLinecap="round" strokeLinejoin="round">
                    <path d="m5 12.5 4.5 4.5L19 7.5" />
                  </svg>
                )}
              </button>
            ))}

            {onBrowseLibrary && (
              <>
                <div className="fx-dd-sep" />
                <button className="fx-dd-item browse" onClick={() => { setOpen(false); onBrowseLibrary(); }}>
                  <span className="pv pv-dd-browse">
                    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round">
                      <circle cx="11" cy="11" r="7" />
                      <path d="m20 20-3.5-3.5" />
                    </svg>
                  </span>
                  <span className="nm">Browse Effects Library…</span>
                </button>
              </>
            )}
          </div>
        )}
      </div>

      {/* onboard sub-modes */}
      {effect.kind === "onboard" && (
        <div className="ctrl-row" style={{ alignItems: "flex-start" }}>
          <span className="lbl" style={{ marginTop: 7 }}>Mode</span>
          <div className="seg-ctrl" style={{ flexWrap: "wrap" }}>
            {(Object.keys(ONBOARD_MODE_LABELS) as OnboardMode[]).map((mode) => (
              <button key={mode} className={onboardMode === mode ? "on" : ""} onClick={() => update({ ...effect, mode })}>
                {ONBOARD_MODE_LABELS[mode]}
              </button>
            ))}
          </div>
        </div>
      )}

      {(showColor || showSpeed || showDirection || showRainbow || effect.kind === "custom") && (
        <div style={{ marginTop: 14, borderTop: "1px solid var(--line)", paddingTop: 4 }}>
          {showColor && "color" in effect && (
            <div className="ctrl-row">
              <span className="lbl">Color</span>
              <ColorPickerPopover color={effect.color} onChange={(color) => update({ ...effect, color }, false)} />
            </div>
          )}

          {showSpeed && "speed" in effect && (
            <div className="ctrl-row">
              <span className="lbl">Speed</span>
              <input
                className="slider"
                type="range"
                min={onboardSpeed ? 0 : 0.1}
                max={onboardSpeed ? 4 : 5}
                step={onboardSpeed ? 1 : 0.1}
                value={effect.speed}
                style={{ ["--p" as string]: `${(effect.speed / (onboardSpeed ? 4 : 5)) * 100}%` }}
                onChange={(e) => update({ ...effect, speed: Number(e.target.value) }, false)}
                onPointerUp={flushPending}
                onKeyUp={(e) => { if (e.key.startsWith("Arrow")) flushPending(); }}
              />
              <span className="muted" style={{ width: 40, textAlign: "right" }}>
                {onboardSpeed ? effect.speed : `${effect.speed.toFixed(1)}×`}
              </span>
            </div>
          )}

          {showDirection && "reverse" in effect && (
            <div className="ctrl-row">
              <span className="lbl">Direction</span>
              <div className="seg-ctrl">
                <button className={!reverse ? "on" : ""} onClick={() => update({ ...effect, reverse: false })}>→ Forward</button>
                <button className={reverse ? "on" : ""} onClick={() => update({ ...effect, reverse: true })}>← Reverse</button>
              </div>
            </div>
          )}

          {showRainbow && effect.kind === "onboard" && (
            <div className="ctrl-row spread">
              <span className="lbl">Rainbow</span>
              <button className={`toggle ${effect.rainbow ? "on" : ""}`} onClick={() => update({ ...effect, rainbow: !effect.rainbow })} />
            </div>
          )}

          {effect.kind === "onboard" && (
            <p className="muted" style={{ margin: "4px 0 0" }}>
              Runs as a firmware effect on the device itself. The on-screen preview animates an approximation
              (Reactive is shown as simulated key flashes) — close, but not phase-synced to the board.
            </p>
          )}
          {effect.kind === "custom" && (
            <p className="muted" style={{ margin: "4px 0 0" }}>
              Click keys or LEDs above to paint them with the selected color.
            </p>
          )}
        </div>
      )}
    </div>
  );
}
