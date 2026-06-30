import { useEffect, useState } from "react";
import { gpuTelemetry, type GpuTelemetry } from "../lib/api";

/** Poll cadence for live GPU telemetry. Matches GPU-Z-class refresh; cheap. */
const POLL_MS = 1500;

/**
 * One telemetry tile. `value` is pre-formatted text (or "—" when unknown);
 * `fill` is the bar fill 0..1, scaled per-metric against a sensible full-scale.
 */
type Tile = { label: string; unit: string; value: string; fill: number };

const clamp01 = (n: number) => Math.max(0, Math.min(1, n));

/** Build the four tiles from a snapshot, picking full-scale per metric. */
function tiles(t: GpuTelemetry | null): Tile[] {
  const fmt = (v: number | null | undefined, d = 0) =>
    v == null ? "—" : v.toFixed(d);
  return [
    {
      label: "GPU Temp",
      unit: "°C",
      value: fmt(t?.tempC),
      fill: clamp01((t?.tempC ?? 0) / 100), // 0..100 °C
    },
    {
      label: "Core Clock",
      unit: "MHz",
      value: fmt(t?.coreClockMhz),
      fill: clamp01((t?.coreClockMhz ?? 0) / 3000), // ~3 GHz boost ceiling
    },
    {
      label: "Fan Speed",
      unit: "%",
      value: fmt(t?.fanPct),
      fill: clamp01((t?.fanPct ?? 0) / 100),
    },
    {
      label: "Power",
      unit: "W",
      value: fmt(t?.powerW),
      fill: clamp01((t?.powerW ?? 0) / 400), // 5080 TGP ~360 W, 400 headroom
    },
  ];
}

/**
 * Live GPU telemetry bar (temp · clock · fan · power) over NVML. Polls while
 * mounted — i.e. while the GPU page is open. Degrades to dashes when NVML is
 * unavailable (no NVIDIA driver) or a given metric isn't reported.
 */
export function GpuTelemetryBar() {
  const [telem, setTelem] = useState<GpuTelemetry | null>(null);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const t = await gpuTelemetry();
        if (!alive) return;
        setTelem(t);
      } catch {
        if (!alive) return;
        setTelem(null);
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, []);

  const available = telem?.available ?? false;

  return (
    <div className="card" style={{ marginBottom: 16 }}>
      <div className="card-pad">
        <div className="gpu-telem">
          {tiles(available ? telem : null).map((t) => (
            <div key={t.label}>
              <div className="muted">{t.label}</div>
              <div className="gv">
                {t.value}
                <small> {t.unit}</small>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
