import { useEffect, useRef, useState } from "react";
import type {
  Color,
  EffectConfig,
  EffectKind,
  OnboardMode,
} from "../../types/device";
import { ColorPickerPopover } from "./ColorPickerPopover";

const EFFECT_LABELS: Record<EffectKind, string> = {
  static: "Static",
  breathing: "Breathing",
  rainbow_wave: "Rainbow Wave",
  color_cycle: "Color Cycle",
  custom: "Custom",
  onboard: "Onboard",
};

/** Preview swatch class per effect kind (animated, accent-tinted). */
const EFFECT_PV: Record<EffectKind, string> = {
  static: "pv-static",
  breathing: "pv-breathe",
  rainbow_wave: "pv-wave",
  color_cycle: "pv-cycle",
  custom: "pv-custom",
  onboard: "pv-onboard",
};

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

const DEFAULT_COLOR: Color = { r: 91, g: 140, b: 255 };

function defaultEffect(kind: EffectKind, prev?: EffectConfig): EffectConfig {
  const prevColor =
    prev &&
    (prev.kind === "static" || prev.kind === "breathing" || prev.kind === "onboard")
      ? prev.color
      : DEFAULT_COLOR;
  const prevSpeed = prev && "speed" in prev ? prev.speed : 1.0;
  switch (kind) {
    case "static":
      return { kind, color: prevColor };
    case "breathing":
      return { kind, color: prevColor, speed: prevSpeed };
    case "rainbow_wave":
      return { kind, speed: prevSpeed, reverse: false };
    case "color_cycle":
      return { kind, speed: prevSpeed };
    case "custom":
      return { kind };
    case "onboard":
      return { kind, mode: "wave", color: prevColor, rainbow: true, speed: 2, reverse: false };
  }
}

interface Props {
  effects: string[];
  /** Current effect (per-device mode). Omit for global mode. */
  value?: EffectConfig;
  onApply: (effect: EffectConfig) => void;
}

export function EffectPanel({ effects, value, onApply }: Props) {
  const [effect, setEffect] = useState<EffectConfig>(
    value ?? { kind: "rainbow_wave", speed: 1.0, reverse: false },
  );
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

  const kinds = effects.filter((e): e is EffectKind => e in EFFECT_LABELS);

  // Which controls apply to the current effect.
  const onboardMode = effect.kind === "onboard" ? effect.mode : null;
  const showColor =
    effect.kind === "static" ||
    effect.kind === "breathing" ||
    (effect.kind === "onboard" && !effect.rainbow && !ONBOARD_COLORLESS.includes(effect.mode));
  const showSpeed =
    effect.kind === "breathing" ||
    effect.kind === "rainbow_wave" ||
    effect.kind === "color_cycle" ||
    (effect.kind === "onboard" && !ONBOARD_SPEEDLESS.includes(effect.mode));
  const showDirection =
    effect.kind === "rainbow_wave" ||
    (effect.kind === "onboard" && ONBOARD_DIRECTIONAL.includes(effect.mode));
  const showRainbow =
    effect.kind === "onboard" && !ONBOARD_COLORLESS.includes(effect.mode);
  const reverse = (effect.kind === "rainbow_wave" || effect.kind === "onboard") ? effect.reverse : false;
  const onboardSpeed = effect.kind === "onboard";

  return (
    <div>
      {/* effect tiles */}
      <div className="effect-grid">
        {kinds.map((kind) => (
          <button key={kind} className={`fx ${effect.kind === kind ? "on" : ""}`} onClick={() => selectKind(kind)}>
            <div className={`pv ${EFFECT_PV[kind]}`} />
            <div className="nm">{EFFECT_LABELS[kind]}</div>
          </button>
        ))}
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

          {showDirection && (
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
