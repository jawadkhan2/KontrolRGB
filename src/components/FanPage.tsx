import { useEffect, useMemo, useRef, useState } from "react";
import { useFans, interpolateCurve } from "../store/fans";
import type { CurvePoint, FanProfile, TempSourceKey, FanMode } from "../store/fans";
import type { ChannelReading, FanMapping, SweepProgress, TempReading } from "../lib/fanApi";
import { gpuTelemetry } from "../lib/api";

const ACCENT = "#4cc2f0";

/** Backend label for the pump header — shown read-only, BIOS-controlled. */
const PUMP_LABEL = "Pump Fan";

const TEMP_SOURCE_LABELS: Record<string, string> = {
  cpu: "CPU",
  aux0: "System",
  aux1: "CPU Diode",
  aux2: "Aux 2",
  aux3: "Aux 3",
  gpu: "GPU",
};

// ─── Curve editor (linear interpolation, matches the backend) ────────────────

const ML = 48, MR = 20, MT = 20, MB = 44;
const CW = 420, CH = 180;
const TW = ML + CW + MR, TH = MT + CH + MB;
const T_MIN = 20, T_MAX = 100;

function toX(t: number) { return ML + ((t - T_MIN) / (T_MAX - T_MIN)) * CW; }
function toY(s: number) { return MT + CH - (s / 100) * CH; }
function fromX(x: number) { return T_MIN + ((x - ML) / CW) * (T_MAX - T_MIN); }
function fromY(y: number) { return 100 - ((y - MT) / CH) * 100; }

function FanCurveEditor({
  points,
  onChange,
  currentTempC,
  minSpeedPct = 0,
  readOnly = false,
}: {
  points: CurvePoint[];
  onChange: (pts: CurvePoint[]) => void;
  currentTempC?: number;
  minSpeedPct?: number;
  readOnly?: boolean;
}) {
  const svgRef = useRef<SVGSVGElement>(null);
  const [dragging, setDragging] = useState<number | null>(null);
  const [hovered, setHovered] = useState<number | null>(null);

  const sorted = [...points].sort((a, b) => a.tempC - b.tempC);

  function clientToSvg(cx: number, cy: number) {
    const svg = svgRef.current;
    if (!svg) return { x: 0, y: 0 };
    const r = svg.getBoundingClientRect();
    return { x: ((cx - r.left) / r.width) * TW, y: ((cy - r.top) / r.height) * TH };
  }

  function clampPoint(rawX: number, rawY: number, idx: number): CurvePoint {
    const prevTemp = idx > 0 ? sorted[idx - 1].tempC + 1 : T_MIN;
    const nextTemp = idx < sorted.length - 1 ? sorted[idx + 1].tempC - 1 : T_MAX;
    const tempC = Math.max(prevTemp, Math.min(nextTemp, Math.round(fromX(rawX))));
    const speedPct = Math.max(0, Math.min(100, Math.round(fromY(rawY))));
    return { tempC, speedPct };
  }

  function handleMove(e: React.PointerEvent) {
    if (dragging === null || readOnly) return;
    const { x, y } = clientToSvg(e.clientX, e.clientY);
    const updated = clampPoint(x, y, dragging);
    onChange(sorted.map((p, i) => (i === dragging ? updated : p)));
  }

  function handleSvgClick(e: React.MouseEvent) {
    if (readOnly || dragging !== null) return;
    const { x, y } = clientToSvg(e.clientX, e.clientY);
    if (x < ML || x > ML + CW || y < MT || y > MT + CH) return;
    const tempC = Math.round(fromX(x));
    const speedPct = Math.max(0, Math.min(100, Math.round(fromY(y))));
    if (sorted.some((p) => Math.abs(p.tempC - tempC) < 3)) return;
    onChange([...sorted, { tempC, speedPct }]);
  }

  function removePoint(idx: number, e: React.MouseEvent) {
    e.stopPropagation();
    if (readOnly || sorted.length <= 2) return;
    onChange(sorted.filter((_, i) => i !== idx));
  }

  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  const linePath = first && last
    ? [
        `M ${ML} ${toY(first.speedPct)}`,
        ...sorted.map((p) => `L ${toX(p.tempC)} ${toY(p.speedPct)}`),
        `L ${ML + CW} ${toY(last.speedPct)}`,
      ].join(" ")
    : "";
  const fillPath = linePath ? `${linePath} L ${ML + CW} ${MT + CH} L ${ML} ${MT + CH} Z` : "";

  const curTempX = currentTempC != null ? toX(Math.max(T_MIN, Math.min(T_MAX, currentTempC))) : null;
  const curSpeedY =
    currentTempC != null && first != null ? toY(interpolateCurve(sorted, currentTempC)) : null;

  const xGrid = [30, 40, 50, 60, 70, 80, 90];
  const yGrid = [25, 50, 75];
  const xLabels = [20, 40, 60, 80, 100];
  const yLabels = [0, 25, 50, 75, 100];

  return (
    <svg
      ref={svgRef}
      viewBox={`0 0 ${TW} ${TH}`}
      className="w-full select-none"
      style={{ touchAction: "none" }}
      onPointerMove={handleMove}
      onPointerUp={() => setDragging(null)}
      onPointerLeave={() => setDragging(null)}
      onClick={handleSvgClick}
    >
      <defs>
        <clipPath id="cc"><rect x={ML} y={MT} width={CW} height={CH} /></clipPath>
        <linearGradient id="cfill" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor={ACCENT} stopOpacity="0.28" />
          <stop offset="100%" stopColor={ACCENT} stopOpacity="0.02" />
        </linearGradient>
      </defs>

      <rect x={ML} y={MT} width={CW} height={CH} fill="#0a0e15" rx="3" />

      {xGrid.map((t) => (
        <line key={t} x1={toX(t)} y1={MT} x2={toX(t)} y2={MT + CH} stroke="#1a2130" strokeWidth="1" />
      ))}
      {yGrid.map((s) => (
        <line key={s} x1={ML} y1={toY(s)} x2={ML + CW} y2={toY(s)} stroke="#1a2130" strokeWidth="1" />
      ))}

      {minSpeedPct > 0 && (
        <rect x={ML} y={toY(minSpeedPct)} width={CW} height={MT + CH - toY(minSpeedPct)}
          fill="#78350f" fillOpacity="0.18" clipPath="url(#cc)" />
      )}

      {fillPath && <path d={fillPath} fill="url(#cfill)" clipPath="url(#cc)" />}
      {linePath && <path d={linePath} stroke={ACCENT} strokeWidth="2" fill="none" clipPath="url(#cc)" />}

      {curTempX != null && (
        <line x1={curTempX} y1={MT} x2={curTempX} y2={MT + CH} stroke="#fbbf24" strokeWidth="1.5" strokeDasharray="4 3" />
      )}
      {curSpeedY != null && curTempX != null && (
        <>
          <line x1={ML} y1={curSpeedY} x2={ML + CW} y2={curSpeedY} stroke="#fbbf24" strokeWidth="1" strokeDasharray="3 4" strokeOpacity="0.5" />
          <circle cx={curTempX} cy={curSpeedY} r="4" fill="#fbbf24" />
        </>
      )}

      <line x1={ML} y1={MT} x2={ML} y2={MT + CH} stroke="#39414f" strokeWidth="1" />
      <line x1={ML} y1={MT + CH} x2={ML + CW} y2={MT + CH} stroke="#39414f" strokeWidth="1" />

      {yLabels.map((s) => (
        <text key={s} x={ML - 6} y={toY(s) + 4} textAnchor="end" fontSize="9" fill="#71717a">{s}%</text>
      ))}
      {xLabels.map((t) => (
        <text key={t} x={toX(t)} y={MT + CH + 16} textAnchor="middle" fontSize="9" fill="#71717a">{t}°</text>
      ))}
      <text x={ML + CW / 2} y={MT + CH + 36} textAnchor="middle" fontSize="9" fill="#52525b">Temperature (°C)</text>

      {sorted.map((p, i) => {
        const cx = toX(p.tempC), cy = toY(p.speedPct);
        const active = hovered === i || dragging === i;
        return (
          <g key={i}>
            <circle cx={cx} cy={cy} r={14} fill="transparent"
              onPointerDown={(e) => {
                if (readOnly) return;
                e.stopPropagation();
                setDragging(i);
                (e.currentTarget as Element).setPointerCapture(e.pointerId);
              }}
              onMouseEnter={() => setHovered(i)}
              onMouseLeave={() => setHovered(null)}
              style={{ cursor: readOnly ? "default" : dragging === i ? "grabbing" : "grab" }} />
            {active && <circle cx={cx} cy={cy} r={9} fill={ACCENT} fillOpacity="0.25" style={{ pointerEvents: "none" }} />}
            <circle cx={cx} cy={cy} r={5} fill={active ? ACCENT : "#e4e4e7"} stroke={active ? "#bfe9fb" : "#52525b"} strokeWidth="1.5" style={{ pointerEvents: "none" }} />
            {hovered === i && !readOnly && sorted.length > 2 && (
              <g transform={`translate(${cx + 9}, ${cy - 9})`} onClick={(e) => removePoint(i, e)} style={{ cursor: "pointer" }}>
                <circle r={5.5} fill="#dc2626" />
                <text textAnchor="middle" dy="3.5" fontSize="9" fill="white" style={{ pointerEvents: "none" }}>×</text>
              </g>
            )}
            {active && (
              <g style={{ pointerEvents: "none" }}>
                <rect x={cx - 28} y={cy - 26} width={56} height={16} rx="3" fill="#11161f" stroke="#39414f" strokeWidth="1" />
                <text x={cx} y={cy - 14} textAnchor="middle" fontSize="9" fill="#e4e4e7">{p.tempC}°C → {p.speedPct}%</text>
              </g>
            )}
          </g>
        );
      })}

      {!readOnly && (
        <text x={ML + CW - 4} y={MT + CH + 36} textAnchor="end" fontSize="8" fill="#39414f">
          click to add · drag to move · hover × to remove
        </text>
      )}
    </svg>
  );
}

// ─── Live RPM sparkline (canvas, rolling buffer) ─────────────────────────────

function Sparkline({ value, max }: { value: number; max: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const buf = useRef<number[]>([]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const dpr = window.devicePixelRatio || 1;
    const N = 90;
    const norm = max > 0 ? Math.max(0, Math.min(1, value / max)) : 0;
    const b = buf.current;
    if (b.length === 0) for (let i = 0; i < N; i++) b.push(norm);
    else { b.push(norm); if (b.length > N) b.shift(); }

    const w = canvas.clientWidth, h = canvas.clientHeight;
    canvas.width = w * dpr; canvas.height = h * dpr;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const cw = canvas.width, ch = canvas.height;
    ctx.clearRect(0, 0, cw, ch);
    const pad = 6 * dpr;
    const x = (j: number) => (j / (N - 1)) * cw;
    const y = (v: number) => ch - pad - v * (ch - 2 * pad);

    ctx.beginPath();
    ctx.moveTo(0, ch);
    b.forEach((v, j) => ctx.lineTo(x(j), y(v)));
    ctx.lineTo(cw, ch); ctx.closePath();
    const g = ctx.createLinearGradient(0, 0, 0, ch);
    g.addColorStop(0, "rgba(76,194,240,.22)");
    g.addColorStop(1, "rgba(76,194,240,0)");
    ctx.fillStyle = g; ctx.fill();

    ctx.beginPath();
    b.forEach((v, j) => (j ? ctx.lineTo(x(j), y(v)) : ctx.moveTo(x(j), y(v))));
    ctx.strokeStyle = ACCENT; ctx.lineWidth = 1.6 * dpr;
    ctx.lineJoin = "round"; ctx.shadowColor = "rgba(76,194,240,.5)"; ctx.shadowBlur = 4 * dpr;
    ctx.stroke(); ctx.shadowBlur = 0;
  }, [value, max]);

  return <canvas ref={canvasRef} />;
}

// ─── Gauge ───────────────────────────────────────────────────────────────────

function Gauge({ temp, pct }: { temp: number | null; pct: number }) {
  const R = 27, C = 2 * Math.PI * R;
  const off = C * (1 - Math.max(0, Math.min(100, pct)) / 100);
  return (
    <div className="gauge">
      <svg width="66" height="66" viewBox="0 0 66 66">
        <circle cx="33" cy="33" r={R} fill="none" stroke="#1c2230" strokeWidth="4" />
        <circle cx="33" cy="33" r={R} fill="none" stroke={ACCENT} strokeWidth="4" strokeLinecap="round"
          strokeDasharray={C} strokeDashoffset={off} />
      </svg>
      <div className="gtxt">
        {/* Round to whole degrees so the 0.5° sensor flicker can't jitter the
            layout or overflow the ring. */}
        <div className="gt">{temp != null ? `${Math.round(temp)}°` : "—"}</div>
        <div className="gp">{Math.round(pct)}%</div>
      </div>
    </div>
  );
}

// ─── Icons ───────────────────────────────────────────────────────────────────

const WarnIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
    <path d="M12 9v4M12 17h.01M10.3 3.9 1.8 18a2 2 0 0 0 1.7 3h17a2 2 0 0 0 1.7-3L13.7 3.9a2 2 0 0 0-3.4 0z" />
  </svg>
);
const CalibIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
    <path d="M12 14a2 2 0 1 0-2-2" /><path d="M3 12a9 9 0 0 1 18 0" /><path d="M16 8l-4 4" />
  </svg>
);
const GearIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);
const ChevDown = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M6 9l6 6 6-6" /></svg>
);
const PencilIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" />
  </svg>
);

function rpmClass(rpm: number) {
  if (rpm === 0) return "fc-rpm zero";
  if (rpm < 400) return "fc-rpm low";
  return "fc-rpm";
}

// ─── Custom dropdown (native <select> popups are unstyleable / offset) ────────

interface SelectItem {
  value: string;
  label: string;
}

function Select({
  value,
  items,
  onChange,
  disabled = false,
  dot = false,
  className = "",
  title,
}: {
  value: string;
  items: SelectItem[];
  onChange: (value: string) => void;
  disabled?: boolean;
  /** Render the leading status dot (used by the temp-sensor pickers). */
  dot?: boolean;
  className?: string;
  /** Tooltip on the wrapper — shows even while the trigger is disabled. */
  title?: string;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDoc = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    window.addEventListener("mousedown", onDoc);
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("mousedown", onDoc);
      window.removeEventListener("keydown", onKey, true);
    };
  }, [open]);

  const current = items.find((i) => i.value === value);

  return (
    <div className={`sel${open ? " open" : ""} ${className}`} ref={ref} title={title}>
      <button type="button" className="sel-trigger" disabled={disabled} onClick={() => setOpen((o) => !o)}>
        {dot && <span className="dot" />}
        <span className="sel-label">{current?.label ?? "—"}</span>
        <svg className="sel-chev" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M6 9l6 6 6-6" /></svg>
      </button>
      {open && (
        <div className="sel-menu" role="listbox">
          {items.map((i) => (
            <button
              key={i.value}
              type="button"
              role="option"
              aria-selected={i.value === value}
              className={`sel-opt${i.value === value ? " on" : ""}`}
              onClick={() => { onChange(i.value); setOpen(false); }}
            >
              {i.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Build temp-sensor dropdown items with live °C suffixes. */
function sensorItems(temps: TempReading[]): SelectItem[] {
  return (Object.keys(TEMP_SOURCE_LABELS) as TempSourceKey[]).map((k) => {
    const r = temps.find((t) => t.key === k);
    return { value: k, label: `${TEMP_SOURCE_LABELS[k]}${r ? ` (${Math.round(r.tempC)}°C)` : ""}` };
  });
}

/** Build preset dropdown items, flagging built-ins. */
function presetItems(profiles: FanProfile[]): SelectItem[] {
  return profiles.map((p) => ({ value: p.id, label: `${p.name}${p.isBuiltin ? " (built-in)" : ""}` }));
}

// ─── Fan card ────────────────────────────────────────────────────────────────

function FanCard({
  mapping,
  rpm,
  profiles,
  mode,
  temps,
  activeProfileId,
  calibrating,
  anySweeping,
  available,
  onSetMode,
  onSetSpeed,
  onSweep,
  onOpenCurve,
}: {
  mapping: FanMapping;
  rpm: number;
  profiles: FanProfile[];
  mode: FanMode;
  temps: TempReading[];
  activeProfileId: string;
  calibrating: boolean;
  anySweeping: boolean;
  available: boolean;
  onSetMode: (m: FanMode) => void;
  onSetSpeed: (pct: number) => void;
  onSweep: () => void;
  onOpenCurve: () => void;
}) {
  const isManual = mode.type === "manual";
  const profile =
    mode.type === "curve"
      ? profiles.find((p) => p.id === mode.profileId)
      : null;
  const previewProfile = profile ?? profiles.find((p) => p.id === activeProfileId) ?? profiles[0];
  const sensorKey = (profile ?? previewProfile)?.tempSource;
  const tempReading =
    temps.find((t) => t.key === sensorKey) ?? temps.find((t) => t.key === "cpu") ?? null;
  const temp = tempReading?.tempC ?? null;

  const curveTarget =
    profile && temp != null
      ? Math.max(mapping.minPwm, Math.min(mapping.maxPwm, interpolateCurve(profile.points, temp)))
      : mapping.minPwm;

  const [sliderPct, setSliderPct] = useState(isManual ? (mode as { pct: number }).pct : curveTarget);
  const lastCurve = useRef(mode.type === "curve" ? mode.profileId : activeProfileId);
  useEffect(() => {
    if (mode.type === "curve") lastCurve.current = mode.profileId;
    else setSliderPct(mode.pct);
  }, [mode]);

  const [open, setOpen] = useState(false);

  const gaugePct = isManual ? sliderPct : curveTarget;
  const calibrated = mapping.measuredStallRpm != null || mapping.measuredMaxRpm != null;
  const maxRpmNorm = mapping.measuredMaxRpm ?? Math.max(rpm * 1.2, 1500);

  const meta = isManual ? "Manual control" : previewProfile?.name ?? "—";

  const fanNames = useFans((s) => s.fanNames);
  const renameFan = useFans((s) => s.renameFan);
  const displayName = fanNames[mapping.rpmChannel] ?? mapping.headerLabel;
  const [editingName, setEditingName] = useState(false);
  const [nameDraft, setNameDraft] = useState(displayName);

  function commitName() {
    renameFan(mapping.rpmChannel, nameDraft);
    setEditingName(false);
  }
  function startEditName() {
    setNameDraft(displayName);
    setEditingName(true);
  }

  function toggleManual() {
    if (isManual) {
      onSetMode({ type: "curve", profileId: lastCurve.current ?? activeProfileId });
    } else {
      onSetMode({ type: "manual", pct: sliderPct });
      onSetSpeed(sliderPct);
    }
  }

  function commitSlider() {
    onSetSpeed(sliderPct);
    onSetMode({ type: "manual", pct: sliderPct });
  }

  return (
    <div className={`fcard${isManual ? " manual" : ""}${open ? " open" : ""}${calibrating ? " calibrating" : ""}`}>
      {calibrating && <div className="calib-tag">Calibrating…</div>}

      <div className="fc-head">
        <div className="fc-id">
          {editingName ? (
            <input
              className="fc-name-edit"
              autoFocus
              maxLength={24}
              value={nameDraft}
              onChange={(e) => setNameDraft(e.target.value)}
              onBlur={commitName}
              onKeyDown={(e) => {
                if (e.key === "Enter") commitName();
                if (e.key === "Escape") { setNameDraft(displayName); setEditingName(false); }
              }}
            />
          ) : (
            <div className="fc-name" onDoubleClick={startEditName}>
              <span className="fc-name-txt">{displayName}</span>
              <button className="fc-name-edit-btn" onClick={startEditName} title="Rename fan">{PencilIcon}</button>
            </div>
          )}
          <div className="fc-meta"><span className="fc-prof-pill">{meta}</span></div>
          <div className={rpmClass(rpm)}>{rpm.toLocaleString()}<small>RPM</small></div>
        </div>
        <div className="fc-side">
          <div className="fc-icons">
            {!calibrated && (
              <button className="ic-btn warn" title="Not calibrated — run calibration" onClick={onSweep} disabled={anySweeping || !available}>
                {WarnIcon}
              </button>
            )}
            <button className="ic-btn" title="Calibrate fan" onClick={onSweep} disabled={anySweeping || !available}>
              {CalibIcon}
            </button>
            <button className="ic-btn" title="Fan curve" onClick={onOpenCurve}>
              {GearIcon}
            </button>
          </div>
          <Gauge temp={temp} pct={gaugePct} />
        </div>
      </div>

      {calibrating ? (
        <div className="calib-status">
          <div className="cs-hint">Measuring stall floor &amp; top RPM…</div>
          <div className="cs-main">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 12a9 9 0 1 1-6.22-8.56" strokeLinecap="round" /></svg>
            Calibrating
          </div>
        </div>
      ) : (
        <div className="fc-graph"><Sparkline value={rpm} max={maxRpmNorm} /></div>
      )}

      {!calibrating && (
        <div className="fc-foot">
          <button className={`mini-toggle${isManual ? " on" : ""}`} onClick={toggleManual} disabled={!available} title="Toggle manual control" />
          {isManual ? (
            <div className="manual-slider">
              <input
                className="slider"
                type="range"
                min={mapping.minPwm}
                max={mapping.maxPwm}
                value={sliderPct}
                style={{ ["--p" as string]: `${((sliderPct - mapping.minPwm) / Math.max(1, mapping.maxPwm - mapping.minPwm)) * 100}%` }}
                onChange={(e) => setSliderPct(Number(e.target.value))}
                onPointerUp={commitSlider}
                onKeyUp={(e) => { if (e.key.startsWith("Arrow")) commitSlider(); }}
              />
              <span className="mbubble">{sliderPct}%</span>
            </div>
          ) : (
            <span className="manual-label">Enable Manual Control</span>
          )}
          <button className="expand" onClick={() => setOpen((o) => !o)} title="Details">{ChevDown}</button>
        </div>
      )}

      <div className="fc-detail">
        <div className="fc-detail-inner"><div className="detail-pad">
          <div className="fc-mini">
            <FanCurveEditor points={previewProfile?.points ?? []} onChange={() => {}} currentTempC={temp ?? undefined} minSpeedPct={mapping.minPwm} readOnly />
          </div>
          <div className="fc-stats">
            <div className="fc-stat"><div className="k">Floor</div><div className="v">{mapping.minPwm}<small>%</small></div></div>
            <div className="fc-stat"><div className="k">Ceiling</div><div className="v">{mapping.maxPwm}<small>%</small></div></div>
            <div className="fc-stat"><div className="k">Stall RPM</div><div className="v">{mapping.measuredStallRpm != null ? mapping.measuredStallRpm.toLocaleString() : "—"}</div></div>
            <div className="fc-stat"><div className="k">Max RPM</div><div className="v">{mapping.measuredMaxRpm != null ? mapping.measuredMaxRpm.toLocaleString() : "—"}</div></div>
          </div>
        </div></div>
      </div>
    </div>
  );
}

// ─── Pump card (read-only, BIOS-controlled) ──────────────────────────────────

function PumpCard({ reading, cpuTemp }: { reading: ChannelReading; cpuTemp: number | null }) {
  return (
    <div className="fcard">
      <div className="fc-head">
        <div className="fc-id">
          <div className="fc-name">{reading.label}</div>
          <div className="fc-dev">BIOS-controlled · not adjustable</div>
          <div className="fc-meta">Follows the BIOS pump curve</div>
          <div className={rpmClass(reading.rpm)}>{reading.rpm.toLocaleString()}<small>RPM</small></div>
        </div>
        <div className="fc-side">
          <div className="fc-icons" style={{ height: 28 }} />
          <Gauge temp={cpuTemp} pct={reading.pwmPct ?? 0} />
        </div>
      </div>
      <div className="fc-graph"><Sparkline value={reading.rpm} max={Math.max(reading.rpm * 1.2, 3000)} /></div>
      <div className="fc-foot">
        <span className="manual-label" style={{ color: "var(--faint)" }}>BIOS curve · locked</span>
      </div>
    </div>
  );
}

// ─── Curve-only editor (profile editing, no fan binding) ─────────────────────

function EditCurveModal({
  profiles,
  temps,
  initialId,
  onClose,
}: {
  profiles: FanProfile[];
  temps: TempReading[];
  initialId: string;
  onClose: () => void;
}) {
  const addProfile = useFans((s) => s.addProfile);
  const updateProfilePoints = useFans((s) => s.updateProfilePoints);
  const setProfileSource = useFans((s) => s.setProfileSource);
  const renameProfile = useFans((s) => s.renameProfile);
  const deleteProfile = useFans((s) => s.deleteProfile);

  const [editId, setEditId] = useState(initialId);
  const src = profiles.find((p) => p.id === editId) ?? profiles[0];
  const isBuiltin = src?.isBuiltin ?? true;

  // Local draft — edits stay here until Save, so Cancel discards, built-ins are
  // never mutated, and nothing is pushed to the running fans mid-edit.
  const [name, setName] = useState(src?.name ?? "Custom");
  const [source, setSource] = useState<TempSourceKey>(src?.tempSource ?? "cpu");
  const [points, setPoints] = useState<CurvePoint[]>(src?.points ?? []);
  const [showDelete, setShowDelete] = useState(false);

  // Reload the draft whenever the selected preset changes. Built-ins seed a
  // "<name> copy" name, since Save forks them to a new editable preset.
  useEffect(() => {
    const p = profiles.find((x) => x.id === editId);
    if (!p) return;
    setName(p.isBuiltin ? `${p.name} copy` : p.name);
    setSource(p.tempSource);
    setPoints(p.points.map((q) => ({ ...q })));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editId]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (showDelete) setShowDelete(false);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showDelete, onClose]);

  const tempReading = temps.find((t) => t.key === source);

  function save() {
    const nm = name.trim() || "Custom Curve";
    if (isBuiltin) {
      // Built-ins are immutable — fork the edit into a new custom preset.
      const id = addProfile(nm);
      updateProfilePoints(id, points);
      setProfileSource(id, source);
    } else {
      if (nm !== src.name) renameProfile(editId, nm);
      updateProfilePoints(editId, points);
      setProfileSource(editId, source);
    }
    onClose();
  }

  function confirmDelete() {
    deleteProfile(editId);
    setShowDelete(false);
    onClose();
  }

  return (
    <div className="scrim" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-head">
          <div className="mt">{PencilIcon} Edit Fan Curve</div>
          <button className="x" onClick={onClose}>✕</button>
        </div>

        <div className="modal-body">
          <div className="ctl-row">
            <div className="preset-row">
              <div className="ctl">
                <div className="cl">Preset</div>
                <Select value={editId} items={presetItems(profiles)} onChange={setEditId} />
              </div>
              <button className="icon-sq" title={isBuiltin ? "Built-in curve — can't delete" : "Delete curve"} disabled={isBuiltin} onClick={() => setShowDelete(true)}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" /><path d="M10 11v6M14 11v6" /></svg>
              </button>
            </div>

            <div className="ctl">
              <div className="cl">Temperature Sensor</div>
              <Select dot value={source} items={sensorItems(temps)} onChange={(v) => setSource(v as TempSourceKey)} />
            </div>
          </div>

          <div className="ctl" style={{ marginTop: 12 }}>
            <div className="cl">Curve Name</div>
            <input
              className="name-input"
              maxLength={24}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Cooling"
            />
          </div>

          <div style={{ marginTop: 12 }}>
            <FanCurveEditor points={points} onChange={setPoints} currentTempC={tempReading?.tempC} />
          </div>
          {isBuiltin && (
            <p className="muted" style={{ textAlign: "center", marginTop: 8 }}>
              Built-in profile · Save creates an editable copy
            </p>
          )}
        </div>

        <div className="modal-foot">
          <div className="sp" />
          <button className="mbtn ghost" onClick={onClose}>Cancel</button>
          <button className="mbtn primary" onClick={save}>{isBuiltin ? "Save as New Preset" : "Save Preset"}</button>
        </div>

        {showDelete && (
          <div className="dlg" onClick={(e) => { if (e.target === e.currentTarget) setShowDelete(false); }}>
            <div className="dlg-card">
              <h4>Delete Curve <button className="x" onClick={() => setShowDelete(false)}>✕</button></h4>
              <p className="muted" style={{ margin: "0 0 4px" }}>
                Delete <strong>{src?.name}</strong>? Fans using it fall back to Balanced. This can't be undone.
              </p>
              <div className="dlg-actions">
                <button className="mbtn ghost" onClick={() => setShowDelete(false)}>Cancel</button>
                <button className="mbtn danger" onClick={confirmDelete}>Delete</button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Fan curve modal ─────────────────────────────────────────────────────────

function CurveModal({
  channel,
  mapping,
  profiles,
  mode,
  temps,
  activeProfileId,
  mappings,
  onClose,
}: {
  channel: number;
  mapping: FanMapping;
  profiles: FanProfile[];
  mode: FanMode;
  temps: TempReading[];
  activeProfileId: string;
  mappings: FanMapping[];
  onClose: () => void;
}) {
  const setFanMode = useFans((s) => s.setFanMode);
  const setProfileSource = useFans((s) => s.setProfileSource);
  const updateProfilePoints = useFans((s) => s.updateProfilePoints);
  const addProfile = useFans((s) => s.addProfile);
  const renameProfile = useFans((s) => s.renameProfile);
  const deleteProfile = useFans((s) => s.deleteProfile);

  const initial = mode.type === "curve" ? mode.profileId : activeProfileId;
  const [editId, setEditId] = useState(initial);
  const [showDlg, setShowDlg] = useState(false);
  const [name, setName] = useState("Cooling");
  const [renaming, setRenaming] = useState(false);
  const [renameDraft, setRenameDraft] = useState("");
  const [showDelete, setShowDelete] = useState(false);

  const editProfile = profiles.find((p) => p.id === editId) ?? profiles[0];
  const isBuiltin = editProfile?.isBuiltin ?? true;
  const sensorKey = editProfile?.tempSource ?? "cpu";
  const tempReading = temps.find((t) => t.key === sensorKey);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      if (showDlg) setShowDlg(false);
      else if (showDelete) setShowDelete(false);
      else if (renaming) setRenaming(false);
      else onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [showDlg, showDelete, renaming, onClose]);

  function pickProfile(id: string) {
    setEditId(id);
    setFanMode(channel, { type: "curve", profileId: id });
  }
  function saveNew() {
    const id = addProfile(name.trim() || "Custom", editId);
    setEditId(id);
    setFanMode(channel, { type: "curve", profileId: id });
    setShowDlg(false);
  }
  function startRename() {
    if (isBuiltin) return;
    setRenameDraft(editProfile.name);
    setRenaming(true);
  }
  function commitRename() {
    const t = renameDraft.trim();
    if (t) renameProfile(editProfile.id, t);
    setRenaming(false);
  }
  function confirmDelete() {
    // Store reassigns affected fans (and the active profile) to Balanced; point
    // the editor there too so it doesn't dangle on the deleted id.
    deleteProfile(editProfile.id);
    setEditId("balanced");
    setShowDelete(false);
  }

  return (
    <div className="scrim" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-head">
          <div className="mt">{GearIcon} Fan Curve — {mapping.headerLabel}</div>
          <button className="x" onClick={onClose}>✕</button>
        </div>

        <div className="modal-body">
          <div className="ctl-row">
            <div className="ctl">
              <div className="cl">Temperature Sensor</div>
              <Select
                dot
                value={sensorKey}
                disabled={isBuiltin}
                title={isBuiltin ? "Built-in curve is locked — clone it or use “Edit curve” to change the sensor" : undefined}
                items={sensorItems(temps)}
                onChange={(v) => setProfileSource(editProfile.id, v as TempSourceKey)}
              />
            </div>

            <div className="preset-row">
              <div className="ctl">
                <div className="cl">Preset</div>
                {renaming ? (
                  <input
                    className="name-input"
                    autoFocus
                    maxLength={24}
                    value={renameDraft}
                    onChange={(e) => setRenameDraft(e.target.value)}
                    onBlur={commitRename}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") commitRename();
                      if (e.key === "Escape") setRenaming(false);
                    }}
                  />
                ) : (
                  <Select value={editId} items={presetItems(profiles)} onChange={pickProfile} />
                )}
              </div>
              <button className="icon-sq" title={isBuiltin ? "Built-in curve — clone to rename" : "Rename curve"} disabled={isBuiltin} onClick={startRename}>
                {PencilIcon}
              </button>
              <button className="icon-sq" title={isBuiltin ? "Clone to edit" : "Duplicate preset"} onClick={() => { setName(`${editProfile.name} copy`); setShowDlg(true); }}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M5 15V5a2 2 0 0 1 2-2h10" /></svg>
              </button>
              <button className="icon-sq" title={isBuiltin ? "Built-in curve — can't delete" : "Delete curve"} disabled={isBuiltin} onClick={() => setShowDelete(true)}>
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M3 6h18M8 6V4a1 1 0 0 1 1-1h6a1 1 0 0 1 1 1v2m2 0v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" /><path d="M10 11v6M14 11v6" /></svg>
              </button>
            </div>
          </div>

          <FanCurveEditor
            points={editProfile?.points ?? []}
            onChange={(pts) => { if (!isBuiltin) updateProfilePoints(editProfile.id, pts); }}
            currentTempC={tempReading?.tempC}
            minSpeedPct={mapping.minPwm}
            readOnly={isBuiltin}
          />
          {isBuiltin && <p className="muted" style={{ textAlign: "center", marginTop: 8 }}>Built-in profile · clone to edit its curve</p>}
        </div>

        <div className="modal-foot">
          <div className="sp" />
          <button className="mbtn ghost" onClick={onClose}>Cancel</button>
          <button className="mbtn" onClick={() => { mappings.forEach((m) => setFanMode(m.rpmChannel, { type: "curve", profileId: editId })); onClose(); }}>Apply to All Fans</button>
          <button className="mbtn primary" onClick={() => { setFanMode(channel, { type: "curve", profileId: editId }); onClose(); }}>Apply to This Fan</button>
        </div>

        {showDlg && (
          <div className="dlg" onClick={(e) => { if (e.target === e.currentTarget) setShowDlg(false); }}>
            <div className="dlg-card">
              <h4>Create Custom Preset <button className="x" onClick={() => setShowDlg(false)}>✕</button></h4>
              <div className="fl">Preset Name</div>
              <input autoFocus maxLength={24} value={name} onChange={(e) => setName(e.target.value)} onKeyDown={(e) => { if (e.key === "Enter") saveNew(); }} placeholder="e.g. Cooling" />
              <div className="dlg-actions">
                <button className="mbtn ghost" onClick={() => setShowDlg(false)}>Cancel</button>
                <button className="mbtn primary" onClick={saveNew}>Save Preset</button>
              </div>
            </div>
          </div>
        )}

        {showDelete && (
          <div className="dlg" onClick={(e) => { if (e.target === e.currentTarget) setShowDelete(false); }}>
            <div className="dlg-card">
              <h4>Delete Curve <button className="x" onClick={() => setShowDelete(false)}>✕</button></h4>
              <p className="muted" style={{ margin: "0 0 4px" }}>
                Delete <strong>{editProfile?.name}</strong>? Fans using it fall back to Balanced. This can't be undone.
              </p>
              <div className="dlg-actions">
                <button className="mbtn ghost" onClick={() => setShowDelete(false)}>Cancel</button>
                <button className="mbtn danger" onClick={confirmDelete}>Delete</button>
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

// ─── Create-custom-fan-curve modal ──────────────────────────────────────────

function CustomCurveModal({
  temps,
  onClose,
}: {
  temps: TempReading[];
  onClose: () => void;
}) {
  const addProfile = useFans((s) => s.addProfile);
  const updateProfilePoints = useFans((s) => s.updateProfilePoints);
  const setProfileSource = useFans((s) => s.setProfileSource);

  const [name, setName] = useState("My Curve");
  const [source, setSource] = useState<TempSourceKey>("cpu");
  const [points, setPoints] = useState<CurvePoint[]>([
    { tempC: 30, speedPct: 30 },
    { tempC: 50, speedPct: 45 },
    { tempC: 65, speedPct: 65 },
    { tempC: 80, speedPct: 90 },
    { tempC: 90, speedPct: 100 },
  ]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const tempReading = temps.find((t) => t.key === source);

  function save() {
    // addProfile clones from Balanced and selects it; overwrite with our curve
    // and chosen sensor so it lands in the profile menu ready to apply.
    const id = addProfile(name.trim() || "Custom Curve");
    updateProfilePoints(id, points);
    setProfileSource(id, source);
    onClose();
  }

  return (
    <div className="scrim" onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}>
      <div className="modal">
        <div className="modal-head">
          <div className="mt">{GearIcon} Create Custom Fan Curve</div>
          <button className="x" onClick={onClose}>✕</button>
        </div>

        <div className="modal-body">
          <div className="ctl-row">
            <div className="ctl">
              <div className="cl">Curve Name</div>
              <input
                className="name-input"
                autoFocus
                maxLength={24}
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. GPU Aggressive"
              />
            </div>
            <div className="ctl">
              <div className="cl">Temperature Sensor</div>
              <Select dot value={source} items={sensorItems(temps)} onChange={(v) => setSource(v as TempSourceKey)} />
            </div>
          </div>

          <FanCurveEditor points={points} onChange={setPoints} currentTempC={tempReading?.tempC} />
        </div>

        <div className="modal-foot">
          <div className="sp" />
          <button className="mbtn ghost" onClick={onClose}>Cancel</button>
          <button className="mbtn primary" onClick={save}>Save Curve</button>
        </div>
      </div>
    </div>
  );
}

// ─── Calibration progress modal ──────────────────────────────────────────────

function CalibrationModal({
  mapping, progress, rpm, onStop, stopping,
}: {
  mapping: FanMapping;
  progress?: SweepProgress;
  rpm: number;
  onStop: () => void;
  stopping: boolean;
}) {
  const pct = progress?.pct ?? 100;
  const settling = progress?.phase === "settling";
  // Polling is suspended during a sweep, so `rpm` is the frozen pre-sweep tach.
  // Track the last live RPM the sweep itself reported and show that instead, so
  // the number tracks the fan ramping rather than sticking at the old value.
  const lastLive = useRef<number | null>(null);
  useEffect(() => {
    if (progress && progress.rpm > 0) lastLive.current = progress.rpm;
  }, [progress?.rpm]);
  const liveRpm = lastLive.current ?? rpm;

  return (
    <div className="scrim">
      <div className="modal" style={{ width: "22rem" }}>
        <div className="modal-body">
          <div className="flex items-center gap-3">
            <svg viewBox="0 0 24 24" className="h-5 w-5 animate-spin" style={{ color: ACCENT }} fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M21 12a9 9 0 1 1-6.22-8.56" strokeLinecap="round" />
            </svg>
            <div>
              <h3 className="text-base font-bold">Calibrating {mapping.headerLabel}</h3>
              <p className="text-xs text-faint">Measuring stall floor &amp; top RPM</p>
            </div>
          </div>

          <div className="mt-5 grid grid-cols-2 gap-3">
            <div className="rounded-xl bg-panel-2 p-3 text-center">
              <div className="text-3xl font-bold tabular-nums text-text">{liveRpm.toLocaleString()}</div>
              <div className="mt-0.5 text-xs text-faint">RPM</div>
            </div>
            <div className="rounded-xl bg-panel-2 p-3 text-center">
              <div className="text-3xl font-bold tabular-nums" style={{ color: ACCENT }}>{pct}%</div>
              <div className="mt-0.5 text-xs text-faint">duty</div>
            </div>
          </div>

          <div className="mt-4 h-1.5 w-full overflow-hidden rounded-full bg-panel-2">
            <div className="h-full rounded-full transition-all duration-300" style={{ width: `${pct}%`, background: ACCENT }} />
          </div>

          <div className="mt-4 flex items-center justify-center gap-2 text-xs">
            <span className={`h-1.5 w-1.5 rounded-full ${settling ? "bg-warn animate-pulse" : "bg-good"}`} />
            <span className="text-dim">
              {progress ? (settling ? "Settling… waiting for RPM to stabilize" : "Measuring RPM at this duty") : "Starting sweep…"}
            </span>
          </div>

          <div className="mt-4 rounded-lg border border-amber-700/40 bg-amber-950/30 px-3 py-2 text-[11px] leading-relaxed text-amber-200/80">
            This usually takes under a minute. Each duty step waits only until the fan's RPM settles, then moves on, so the stall floor is measured accurately — don't close the app.
          </div>

          <button onClick={onStop} disabled={stopping}
            className="mt-3 w-full rounded-lg bg-red-600/90 px-4 py-2 text-sm font-semibold text-white transition-colors hover:bg-red-500 disabled:cursor-not-allowed disabled:opacity-50">
            {stopping ? "Stopping…" : "■ Stop calibration"}
          </button>
          <p className="mt-3 text-center text-[11px] text-faint">Fan returns to the BIOS curve automatically when finished or stopped.</p>
        </div>
      </div>
    </div>
  );
}

// ─── Main page ───────────────────────────────────────────────────────────────

export function FanPage() {
  const status = useFans((s) => s.status);
  const readings = useFans((s) => s.readings);
  const temps = useFans((s) => s.temps);
  const sweepingChannel = useFans((s) => s.sweepingChannel);
  const sweepProgress = useFans((s) => s.sweepProgress);
  const profiles = useFans((s) => s.profiles);
  const activeProfileId = useFans((s) => s.activeProfileId);
  const fanModes = useFans((s) => s.fanModes);

  const sweep = useFans((s) => s.sweep);
  const cancelSweep = useFans((s) => s.cancelSweep);
  const applyProfileToAll = useFans((s) => s.applyProfileToAll);
  const setFanMode = useFans((s) => s.setFanMode);
  const setSpeed = useFans((s) => s.setSpeed);

  const [stoppingCalibration, setStoppingCalibration] = useState(false);
  const [curveChannel, setCurveChannel] = useState<number | null>(null);
  // Toolbar "Edit curve": profile-only editor, not bound to any fan.
  const [editCurveOpen, setEditCurveOpen] = useState(false);
  const [customOpen, setCustomOpen] = useState(false);

  // Live GPU die temp (NVML), so a fan curve can be driven off the GPU. Polled
  // here for the picker preview; the backend control loop reads it independently.
  const [gpuTempC, setGpuTempC] = useState<number | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = async () => {
      try {
        const t = await gpuTelemetry();
        if (alive) setGpuTempC(t.available ? t.tempC : null);
      } catch {
        if (alive) setGpuTempC(null);
      }
    };
    void tick();
    const id = window.setInterval(() => void tick(), 1500);
    return () => { alive = false; clearInterval(id); };
  }, []);

  // The fan control loop runs app-wide (see App.tsx) so it survives navigation;
  // this page only reads the store it populates. No polling started here.

  useEffect(() => {
    if (sweepingChannel === null) setStoppingCalibration(false);
  }, [sweepingChannel]);

  const mappings = status?.mappings ?? [];
  // Expose GPU as a selectable temp source alongside the chip sensors.
  const tempsAll = useMemo(
    () => (gpuTempC != null ? [...temps, { key: "gpu", label: "GPU", tempC: gpuTempC }] : temps),
    [temps, gpuTempC],
  );
  const rpmByChannel = new Map(readings.map((r) => [r.index, r.rpm]));
  const pumpReading = readings.find((r) => r.label === PUMP_LABEL);
  const cpuTemp = temps.find((t) => t.key === "cpu")?.tempC ?? null;
  const available = !!status?.available;

  const curveMapping = curveChannel != null ? mappings.find((m) => m.rpmChannel === curveChannel) : null;

  async function calibrateAll() {
    for (const m of mappings) {
      // eslint-disable-next-line no-await-in-loop
      await sweep(m.rpmChannel);
    }
  }

  return (
    <main className="main">
      <div className="fan-page">
        {/* toolbar */}
        <div className="fan-bar">
          <div className="profile-pick-wrap" title="Apply profile to all fans">
            <Select
              className="sel-toolbar"
              value={activeProfileId}
              items={profiles.map((p) => ({ value: p.id, label: p.name }))}
              onChange={applyProfileToAll}
            />
          </div>
          <button className="new-profile" title="Edit the selected curve" onClick={() => setEditCurveOpen(true)}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d="M12 20h9" /><path d="M16.5 3.5a2.1 2.1 0 0 1 3 3L7 19l-4 1 1-4z" /></svg>
            Edit curve
          </button>
          <button className="new-profile" onClick={() => setCustomOpen(true)}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"><path d="M3 17l5-6 4 4 5-7M3 21h18" /></svg>
            Create custom fan curve
          </button>
          <button className="calib-all" onClick={() => void calibrateAll()} disabled={!available || sweepingChannel !== null || mappings.length === 0}>
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M12 2v4M12 18v4M2 12h4M18 12h4M5 5l2.5 2.5M16.5 16.5L19 19M19 5l-2.5 2.5M7.5 16.5L5 19" /></svg>
            Calibrate all
          </button>

          <div className="bar-spacer" />
        </div>

        {/* banners */}
        {status && !status.available && (
          <div className="fan-banner warn">
            <div className="bt">Fan control unavailable</div>
            <div>{status.detail}</div>
            <div style={{ opacity: 0.8, marginTop: 6 }}>
              KontrolRGB needs the PawnIO driver installed (from pawnio.eu) and to run as Administrator to access the NCT6687D-R sensor chip. Without it, the BIOS keeps full control.
            </div>
          </div>
        )}
        {status?.available && mappings.length === 0 && (
          <div className="fan-banner info">
            <div className="bt">No controllable fans yet</div>
            <div>KontrolRGB detects connected fans automatically on startup. If none show up, make sure other fan/RGB software is closed — detection can be toggled in Settings.</div>
          </div>
        )}

        {/* fan grid */}
        <div className="fan-grid">
          {mappings.map((m) => (
            <FanCard
              key={m.rpmChannel}
              mapping={m}
              rpm={rpmByChannel.get(m.rpmChannel) ?? 0}
              profiles={profiles}
              mode={fanModes[m.rpmChannel] ?? { type: "curve", profileId: "balanced" }}
              temps={tempsAll}
              activeProfileId={activeProfileId}
              calibrating={sweepingChannel === m.rpmChannel}
              anySweeping={sweepingChannel !== null}
              available={available}
              onSetMode={(mode) => setFanMode(m.rpmChannel, mode)}
              onSetSpeed={(pct) => void setSpeed(m.rpmChannel, pct)}
              onSweep={() => void sweep(m.rpmChannel)}
              onOpenCurve={() => setCurveChannel(m.rpmChannel)}
            />
          ))}
          {status?.available && pumpReading && <PumpCard reading={pumpReading} cpuTemp={cpuTemp} />}
        </div>
      </div>

      {/* curve modal */}
      {curveMapping && curveChannel != null && (
        <CurveModal
          channel={curveChannel}
          mapping={curveMapping}
          profiles={profiles}
          mode={fanModes[curveChannel] ?? { type: "curve", profileId: "balanced" }}
          temps={tempsAll}
          activeProfileId={activeProfileId}
          mappings={mappings}
          onClose={() => setCurveChannel(null)}
        />
      )}

      {/* curve-only editor (toolbar "Edit curve") */}
      {editCurveOpen && (
        <EditCurveModal
          profiles={profiles}
          temps={tempsAll}
          initialId={activeProfileId}
          onClose={() => setEditCurveOpen(false)}
        />
      )}

      {/* create-custom-fan-curve modal */}
      {customOpen && (
        <CustomCurveModal temps={tempsAll} onClose={() => setCustomOpen(false)} />
      )}

      {/* live calibration modal */}
      {sweepingChannel !== null && (() => {
        const m = mappings.find((x) => x.rpmChannel === sweepingChannel);
        return m ? (
          <CalibrationModal
            mapping={m}
            progress={sweepProgress[sweepingChannel]}
            rpm={rpmByChannel.get(sweepingChannel) ?? 0}
            stopping={stoppingCalibration}
            onStop={() => { setStoppingCalibration(true); void cancelSweep(); }}
          />
        ) : null;
      })()}
    </main>
  );
}
