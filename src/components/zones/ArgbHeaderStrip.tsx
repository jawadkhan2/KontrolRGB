import { useRef, useState } from "react";
import { useDevices } from "../../store/devices";
import type { ZoneInfo } from "../../types/device";
import { cssColor } from "../../types/device";

/** Compact ARGB header row (redesign .hdr-strip): name + live inline LED bars +
 *  Pulse/identify, matching the motherboard mockup. Painting works in Custom. */
export function ArgbHeaderStrip({ deviceId, zone }: { deviceId: string; zone: ZoneInfo }) {
  const colors = useDevices((s) => s.frames[deviceId]?.[zone.id]);
  const paintLed = useDevices((s) => s.paintLed);
  const identifyZone = useDevices((s) => s.identifyZone);
  const resizeZone = useDevices((s) => s.resizeZone);
  const renameZone = useDevices((s) => s.renameZone);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(zone.name);
  const cancelled = useRef(false);

  const step = (d: number) => {
    const next = Math.min(zone.max_leds, Math.max(zone.min_leds, zone.led_count + d));
    if (next !== zone.led_count) resizeZone(deviceId, zone.id, next);
  };
  const commit = () => {
    setEditing(false);
    if (cancelled.current) return;
    const n = draft.trim();
    if (n && n !== zone.name) renameZone(deviceId, zone.id, n);
  };

  return (
    <div className="hdr-strip">
      <div className="hdr-name">
        {editing ? (
          <input
            autoFocus
            className="hdr-rename"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") { cancelled.current = true; setEditing(false); }
            }}
          />
        ) : (
          <button
            className="nm-btn"
            title="Rename header"
            onClick={() => { setDraft(zone.name); cancelled.current = false; setEditing(true); }}
          >
            {zone.name}
          </button>
        )}
        <div className="muted">{zone.led_count} LEDs · 5V</div>
      </div>

      <div className="leds">
        {Array.from({ length: zone.led_count }, (_, i) => {
          const c = colors?.[i];
          return (
            <button
              key={i}
              className={`led ${c ? "" : "off"}`}
              style={{ ["--lc" as string]: c ? cssColor(c) : "#2a2d36" }}
              onMouseDown={() => paintLed(deviceId, zone.id, i)}
              onMouseEnter={(e) => { if (e.buttons === 1) paintLed(deviceId, zone.id, i); }}
              title={`LED ${i + 1}`}
            />
          );
        })}
      </div>

      <div className="hdr-actions">
        {zone.resizable && (
          <>
            <button className="btn icon" onClick={() => step(-1)} title="Fewer LEDs">−</button>
            <button className="btn icon" onClick={() => step(1)} title="More LEDs">+</button>
          </>
        )}
        <button className="btn" onClick={() => identifyZone(deviceId, zone.id)} title="Pulse this strip to identify it">◉ Pulse</button>
      </div>
    </div>
  );
}
