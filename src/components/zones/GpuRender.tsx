import { useDevices } from "../../store/devices";
import type { Color, DeviceInfo } from "../../types/device";
import { cssColor } from "../../types/device";

/** Average a zone's live colors into one swatch (the fan rings share one hue). */
function avg(colors?: Color[]): Color {
  if (!colors || colors.length === 0) return { r: 132, g: 204, b: 22 };
  const s = colors.reduce((a, c) => ({ r: a.r + c.r, g: a.g + c.g, b: a.b + c.b }), { r: 0, g: 0, b: 0 });
  const n = colors.length;
  return { r: Math.round(s.r / n), g: Math.round(s.g / n), b: Math.round(s.b / n) };
}

const R = 70;
const HR = R * 0.3;
const BLADE =
  `M ${-R * 0.06} ${-HR}` +
  ` C ${-R * 0.24} ${-R * 0.5}, ${-R * 0.34} ${-R * 0.74}, ${-R * 0.09} ${-R * 0.92}` +
  ` C ${R * 0.1} ${-R * 0.8}, ${R * 0.2} ${-R * 0.52}, ${R * 0.14} ${-HR} Z`;

/** One WindForce fan: dark Hawk-blade assembly wrapped in a live RGB ring. */
function Fan({ cx, cy, reverse, dur, ring }: { cx: number; cy: number; reverse: boolean; dur: number; ring: string }) {
  return (
    <g transform={`translate(${cx} ${cy})`}>
      <circle r={R + 9} fill="#0c0e11" />
      <circle r={R + 2} fill="none" stroke={ring} strokeWidth="3.5" filter="url(#gpuGlow)" />
      <circle r={R + 2} fill="none" stroke={ring} strokeWidth="2" />
      <circle r={R - 2} fill="#101317" />
      <g className={`fan-spin ${reverse ? "rev" : ""}`} style={{ ["--dur" as string]: `${dur}s` }}>
        <circle r={R - 3} fill="none" stroke="#23272e" strokeWidth="3" />
        {Array.from({ length: 9 }, (_, i) => (
          <path key={i} d={BLADE} transform={`rotate(${i * 40})`} fill="url(#blade)" stroke="#0c0e11" strokeWidth="0.6" />
        ))}
        <circle r={HR} fill="#1a1d23" stroke="#2c313a" strokeWidth="1" />
        <path d={`M ${-HR * 0.5} 0 A ${HR * 0.5} ${HR * 0.5} 0 1 1 ${HR * 0.5} 0`} fill="none" stroke="#3a4049" strokeWidth="1.4" />
        <circle r="2.5" fill="#2c313a" />
      </g>
    </g>
  );
}

/**
 * Stylized render of the Gigabyte RTX 5080 Gaming OC (triple WindForce). The two
 * onboard zones — Logo (fan-ring halos) and Side Bar (top edge RGB bar) — are
 * painted from the device's live frame, with counter-rotating centre fan.
 */
export function GpuRender({ deviceId, device }: { deviceId: string; device: DeviceInfo }) {
  const frames = useDevices((s) => s.frames[deviceId]);
  const logoZone = device.zones.find((z) => z.id === "logo") ?? device.zones[0];
  const sideZone = device.zones.find((z) => z.id === "side") ?? device.zones[1] ?? device.zones[0];
  const logoColors = logoZone && frames?.[logoZone.id];
  const sideColors = (sideZone && frames?.[sideZone.id]) || [];
  const ring = cssColor(avg(logoColors));

  const W = 560;
  const H = 232;
  const cxs = [108, 280, 452];

  const barStops =
    sideColors.length > 0
      ? sideColors.map((c, i) => (
          <stop key={i} offset={`${(i / Math.max(1, sideColors.length - 1)) * 100}%`} stopColor={cssColor(c)} />
        ))
      : [
          <stop key="a" offset="0%" stopColor="#ff5a4d" />,
          <stop key="b" offset="25%" stopColor="#fbbf24" />,
          <stop key="c" offset="50%" stopColor="#84cc16" />,
          <stop key="d" offset="75%" stopColor="#22d3ee" />,
          <stop key="e" offset="100%" stopColor="#5b8cff" />,
        ];

  return (
    <div className="gpu-stage">
      <svg viewBox={`0 0 ${W} ${H}`} xmlns="http://www.w3.org/2000/svg">
        <defs>
          <linearGradient id="shroud" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#22262d" />
            <stop offset="1" stopColor="#101318" />
          </linearGradient>
          <radialGradient id="blade" cx="50%" cy="30%" r="80%">
            <stop offset="0" stopColor="#3a3f48" />
            <stop offset="60%" stopColor="#23272e" />
            <stop offset="100%" stopColor="#14171c" />
          </radialGradient>
          <linearGradient id="topbarLive">{barStops}</linearGradient>
          <filter id="gpuGlow" x="-50%" y="-50%" width="200%" height="200%">
            <feGaussianBlur stdDeviation="4" />
          </filter>
        </defs>

        {/* top RGB edge bar (live, sits on the card top like RGB Fusion) */}
        <rect x="22" y="2" width="516" height="7" rx="3.5" fill="url(#topbarLive)" filter="url(#gpuGlow)" />
        <rect x="22" y="3" width="516" height="5" rx="2.5" fill="url(#topbarLive)" />

        {/* shroud body with angular armor accents */}
        <rect x="8" y="12" width="544" height="212" rx="16" fill="url(#shroud)" stroke="rgba(255,255,255,.05)" />
        <path d="M8 150 L180 120 L360 150 L552 118 L552 224 L8 224 Z" fill="#0e1116" opacity="0.5" />
        <path d="M360 30 L470 22 L552 40 L552 60 L360 50 Z" fill="#191d23" opacity="0.7" />
        <line x1="200" y1="200" x2="540" y2="200" stroke="#3a4049" strokeWidth="1" opacity="0.4" />
        <text x="544" y="216" textAnchor="end" fontFamily="Inter" fontSize="9" fontWeight="700" letterSpacing="2" fill="#3f444d">
          WINDFORCE
        </text>

        <Fan cx={cxs[0]} cy={118} reverse={false} dur={2.4} ring={ring} />
        <Fan cx={cxs[1]} cy={118} reverse dur={2.0} ring={ring} />
        <Fan cx={cxs[2]} cy={118} reverse={false} dur={2.4} ring={ring} />
      </svg>
      <div className="gpu-cap">
        <div className="item"><span className="d" style={{ color: ring }} /> Three-ring fan halos</div>
        <div className="item"><span className="d" style={{ color: cssColor(avg(sideColors)) }} /> Top edge RGB bar</div>
        <div className="item"><span className="d" style={{ color: "var(--faint)", boxShadow: "none" }} /> Counter-rotating fans</div>
      </div>
    </div>
  );
}
