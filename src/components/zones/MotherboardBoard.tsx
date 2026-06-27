import { useDevices } from "../../store/devices";
import type { Color, DeviceInfo } from "../../types/device";
import { cssColor } from "../../types/device";

/** Average a zone's live colors into one swatch (for the soft underglow). */
function avg(colors?: Color[]): Color {
  if (!colors || colors.length === 0) return { r: 46, g: 50, b: 60 };
  const s = colors.reduce((a, c) => ({ r: a.r + c.r, g: a.g + c.g, b: a.b + c.b }), { r: 0, g: 0, b: 0 });
  const n = colors.length;
  return { r: Math.round(s.r / n), g: Math.round(s.g / n), b: Math.round(s.b / n) };
}

/**
 * Stylized render of the MSI Z890 board. The two onboard RGB zones (right-edge
 * ARGB + PCH underglow) are painted from the device's live frame, so the board
 * reflects whatever the effect engine is actually pushing to the hardware.
 */
export function MotherboardBoard({ deviceId, device }: { deviceId: string; device: DeviceInfo }) {
  const frames = useDevices((s) => s.frames[deviceId]);
  const zone0 = device.zones[0];
  const zone1 = device.zones[1] ?? device.zones[0];
  const edge = (zone0 && frames?.[zone0.id]) || [];
  const pch = cssColor(avg(zone1 && frames?.[zone1.id]));

  // Vertical gradient down the right edge built from the live per-LED colors.
  const edgeStops =
    edge.length > 0
      ? edge.map((c, i) => (
          <stop key={i} offset={`${(i / Math.max(1, edge.length - 1)) * 100}%`} stopColor={cssColor(c)} />
        ))
      : [
          <stop key="a" offset="0%" stopColor="#3a4150" />,
          <stop key="b" offset="100%" stopColor="#2a2f3a" />,
        ];

  return (
    <div className="board-stage">
      <svg viewBox="0 0 300 290" xmlns="http://www.w3.org/2000/svg">
        <defs>
          <pattern id="trace" width="14" height="14" patternUnits="userSpaceOnUse">
            <path d="M0 7h14M7 0v14" stroke="rgba(120,140,170,.05)" strokeWidth="1" />
          </pattern>
          <linearGradient id="metal" x1="0" y1="0" x2="0" y2="1">
            <stop offset="0" stopColor="#3a3f48" /><stop offset="1" stopColor="#23262d" />
          </linearGradient>
          <linearGradient id="metal2" x1="0" y1="0" x2="1" y2="1">
            <stop offset="0" stopColor="#2c3038" /><stop offset="1" stopColor="#1a1c22" />
          </linearGradient>
          <linearGradient id="edgeLive" x1="0" y1="0" x2="0" y2="1">{edgeStops}</linearGradient>
          <filter id="soft"><feGaussianBlur stdDeviation="3" /></filter>
        </defs>

        {/* PCB */}
        <rect x="6" y="6" width="288" height="278" rx="8" fill="#0c0d11" />
        <rect x="6" y="6" width="288" height="278" rx="8" fill="url(#trace)" />
        <rect x="6" y="6" width="288" height="278" rx="8" fill="none" stroke="rgba(255,255,255,.05)" />

        {/* right-edge ARGB underglow (live) */}
        <rect x="276" y="20" width="10" height="250" rx="5" fill="url(#edgeLive)" filter="url(#soft)" opacity="0.9" />
        <rect x="278" y="22" width="5" height="246" rx="2.5" fill="url(#edgeLive)" />

        {/* I/O cover + top VRM heatsink */}
        <path d="M16 16 H120 L132 28 V70 H16 Z" fill="url(#metal)" />
        <rect x="16" y="16" width="58" height="54" rx="3" fill="url(#metal2)" />
        <text x="45" y="48" textAnchor="middle" fontFamily="Inter" fontSize="9" fontWeight="800" fill="#5a616b">MSI</text>
        <rect x="80" y="22" width="48" height="6" rx="3" fill="#84cc16" opacity="0.55" />
        <g fill="#171a20"><rect x="80" y="34" width="48" height="4" rx="2" /><rect x="80" y="42" width="48" height="4" rx="2" /><rect x="80" y="50" width="48" height="4" rx="2" /><rect x="80" y="58" width="48" height="4" rx="2" /></g>

        {/* left VRM heatsink */}
        <rect x="138" y="16" width="56" height="54" rx="4" fill="url(#metal)" />
        <g fill="#15181d"><rect x="144" y="24" width="44" height="3" rx="1.5" /><rect x="144" y="31" width="44" height="3" rx="1.5" /><rect x="144" y="38" width="44" height="3" rx="1.5" /><rect x="144" y="45" width="44" height="3" rx="1.5" /><rect x="144" y="52" width="44" height="3" rx="1.5" /><rect x="144" y="59" width="44" height="3" rx="1.5" /></g>
        <rect x="138" y="65" width="56" height="4" rx="2" fill="#84cc16" opacity="0.5" />

        {/* CPU socket */}
        <rect x="138" y="84" width="60" height="60" rx="3" fill="#16191f" stroke="#33373f" strokeWidth="2" />
        <rect x="146" y="92" width="44" height="44" rx="2" fill="#1d2027" stroke="#3a3f48" />
        <text x="168" y="118" textAnchor="middle" fontFamily="Inter" fontSize="8" fontWeight="700" letterSpacing="1" fill="#3f444d">LGA 1851</text>

        {/* DIMM slots */}
        <g>
          <rect x="212" y="80" width="9" height="92" rx="2" fill="#16181d" stroke="#2a2d34" />
          <rect x="226" y="80" width="9" height="92" rx="2" fill="#16181d" stroke="#2a2d34" />
          <rect x="240" y="80" width="9" height="92" rx="2" fill="#16181d" stroke="#2a2d34" />
          <rect x="254" y="80" width="9" height="92" rx="2" fill="#16181d" stroke="#2a2d34" />
          <rect x="212" y="80" width="9" height="6" fill="#84cc16" opacity="0.6" />
          <rect x="240" y="80" width="9" height="6" fill="#84cc16" opacity="0.6" />
        </g>

        {/* M.2 Shield Frozr heatsinks */}
        <rect x="16" y="86" width="106" height="22" rx="3" fill="url(#metal)" />
        <rect x="22" y="92" width="70" height="3" rx="1.5" fill="#15181d" /><rect x="22" y="99" width="70" height="3" rx="1.5" fill="#15181d" />
        <circle cx="112" cy="97" r="3" fill="#15181d" />
        <rect x="16" y="150" width="150" height="26" rx="3" fill="url(#metal)" />
        <rect x="24" y="158" width="100" height="3.5" rx="1.5" fill="#15181d" /><rect x="24" y="166" width="100" height="3.5" rx="1.5" fill="#15181d" />
        <rect x="150" y="156" width="10" height="14" rx="2" fill="#84cc16" opacity="0.5" />

        {/* PCIe slots */}
        <rect x="16" y="190" width="150" height="11" rx="2" fill="#1b1e24" stroke="#4a5059" strokeWidth="1.4" />
        <rect x="16" y="212" width="120" height="9" rx="2" fill="#16181d" stroke="#2a2d34" />
        <rect x="16" y="230" width="150" height="11" rx="2" fill="#1b1e24" stroke="#4a5059" strokeWidth="1.4" />

        {/* PCH heatsink with live underglow */}
        <rect x="196" y="196" width="78" height="74" rx="6" fill={pch} filter="url(#soft)" opacity="0.4" />
        <path d="M200 200 h70 a4 4 0 0 1 4 4 v58 a4 4 0 0 1 -4 4 h-70 a4 4 0 0 1 -4 -4 v-58 a4 4 0 0 1 4 -4 z" fill="url(#metal2)" />
        <path d="M210 246 l14 -26 l8 14 l10 -18 l14 30 z" fill="#2f343d" />
        <text x="235" y="262" textAnchor="middle" fontFamily="Inter" fontSize="9" fontWeight="800" letterSpacing="2" fill="#646b78">Z890</text>

        {/* JARGB header pins */}
        <g fill="#2a2d34"><rect x="20" y="258" width="3" height="9" rx="1" /><rect x="25" y="258" width="3" height="9" rx="1" /><rect x="30" y="258" width="3" height="9" rx="1" /><rect x="35" y="258" width="3" height="9" rx="1" /></g>
        <text x="44" y="266" fontFamily="Inter" fontSize="7" fill="#4a5059">JARGB</text>
      </svg>
      <div className="board-cap">
        <div className="item"><span className="d" style={{ color: cssColor(avg(edge)) }} /> Right-edge ARGB</div>
        <div className="item"><span className="d" style={{ color: pch }} /> PCH underglow</div>
        <div className="item"><span className="d" style={{ color: "#84cc16" }} /> Heatsink accents</div>
      </div>
    </div>
  );
}
