import { useDevices } from "../../store/devices";
import type { Color, KeyInfo, ZoneInfo } from "../../types/device";
import { cssColor } from "../../types/device";

const UNIT = 44; // px per key unit
const GAP = 3;

interface Props {
  deviceId: string;
  zone: ZoneInfo;
  keys: KeyInfo[];
}

function Key({
  k,
  color,
  onPaint,
}: {
  k: KeyInfo;
  color: Color | undefined;
  onPaint: () => void;
}) {
  const css = color ? cssColor(color) : "#18181b";
  return (
    <button
      onMouseDown={onPaint}
      onMouseEnter={(e) => {
        // held mouse button paints as you sweep across keys
        if (e.buttons === 1) onPaint();
      }}
      className="absolute flex items-center justify-center rounded-[5px] border border-black/40 text-[9px] font-semibold transition-colors duration-75"
      style={{
        left: k.x * UNIT + GAP / 2,
        top: k.y * UNIT + GAP / 2,
        width: k.w * UNIT - GAP,
        height: k.h * UNIT - GAP,
        background: css,
        // Always white legends, lifted off the colored key with a dark halo.
        color: "rgba(255,255,255,0.92)",
        textShadow: "0 1px 2px rgba(0,0,0,0.85), 0 0 1px rgba(0,0,0,0.9)",
        boxShadow: color ? `0 0 8px ${css}66, inset 0 0 4px rgba(0,0,0,0.3)` : undefined,
      }}
    >
      {k.label}
    </button>
  );
}

export function KeyboardLayout({ deviceId, zone, keys }: Props) {
  const colors = useDevices((s) => s.frames[deviceId]?.[zone.id]);
  const paintLed = useDevices((s) => s.paintLed);

  const width = Math.max(...keys.map((k) => k.x + k.w)) * UNIT;
  const height = Math.max(...keys.map((k) => k.y + k.h)) * UNIT;

  return (
    <div className="overflow-x-auto">
      <div
        className="relative rounded-xl bg-black/40 p-3"
        style={{ width: width + 24, height: height + 24 }}
      >
        <div className="relative" style={{ width, height }}>
          {keys.map((k) => (
            <Key
              key={k.led_index}
              k={k}
              color={colors?.[k.led_index]}
              onPaint={() => paintLed(deviceId, zone.id, k.led_index)}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
