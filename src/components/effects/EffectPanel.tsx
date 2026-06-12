import { useEffect, useState } from "react";
import type { Color, EffectConfig, EffectKind } from "../../types/device";
import { ColorPickerPopover } from "./ColorPickerPopover";

const EFFECT_LABELS: Record<EffectKind, string> = {
  static: "Static",
  breathing: "Breathing",
  rainbow_wave: "Rainbow Wave",
  color_cycle: "Color Cycle",
  custom: "Custom",
};

const DEFAULT_COLOR: Color = { r: 139, g: 92, b: 246 };

function defaultEffect(kind: EffectKind, prev?: EffectConfig): EffectConfig {
  const prevColor =
    prev && (prev.kind === "static" || prev.kind === "breathing")
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
  useEffect(() => {
    if (value) setEffect(value);
  }, [value]);

  const update = (next: EffectConfig) => {
    setEffect(next);
    onApply(next);
  };

  const kinds = effects.filter((e): e is EffectKind => e in EFFECT_LABELS);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap gap-1.5">
        {kinds.map((kind) => (
          <button
            key={kind}
            onClick={() => update(defaultEffect(kind, effect))}
            className={`rounded-md px-3 py-1.5 text-xs font-semibold transition-colors ${
              effect.kind === kind
                ? "bg-accent text-white"
                : "bg-panel-2 text-zinc-400 hover:text-zinc-200"
            }`}
          >
            {EFFECT_LABELS[kind]}
          </button>
        ))}
      </div>

      {(effect.kind === "static" || effect.kind === "breathing") && (
        <div className="flex items-center gap-3">
          <span className="w-14 text-xs text-zinc-400">Color</span>
          <ColorPickerPopover
            color={effect.color}
            onChange={(color) => update({ ...effect, color })}
          />
        </div>
      )}

      {(effect.kind === "breathing" ||
        effect.kind === "rainbow_wave" ||
        effect.kind === "color_cycle") && (
        <div className="flex items-center gap-3">
          <span className="w-14 text-xs text-zinc-400">Speed</span>
          <input
            type="range"
            min={0.1}
            max={5}
            step={0.1}
            value={effect.speed}
            onChange={(e) =>
              update({ ...effect, speed: Number(e.target.value) })
            }
            className="flex-1 accent-(--color-accent)"
          />
          <span className="w-8 text-right text-xs tabular-nums text-zinc-400">
            {effect.speed.toFixed(1)}x
          </span>
        </div>
      )}

      {effect.kind === "rainbow_wave" && (
        <label className="flex items-center gap-3 text-xs text-zinc-400">
          <span className="w-14">Reverse</span>
          <input
            type="checkbox"
            checked={effect.reverse}
            onChange={(e) => update({ ...effect, reverse: e.target.checked })}
            className="h-4 w-4 accent-(--color-accent)"
          />
        </label>
      )}

      {effect.kind === "custom" && (
        <p className="text-xs text-zinc-500">
          Click keys or LEDs above to paint them with the selected color.
        </p>
      )}
    </div>
  );
}
