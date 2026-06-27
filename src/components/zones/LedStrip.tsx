import { useRef, useState } from "react";
import { useDevices } from "../../store/devices";
import type { ZoneInfo } from "../../types/device";
import { cssColor } from "../../types/device";

interface Props {
  deviceId: string;
  zone: ZoneInfo;
}

export function LedStrip({ deviceId, zone }: Props) {
  const colors = useDevices((s) => s.frames[deviceId]?.[zone.id]);
  const paintLed = useDevices((s) => s.paintLed);
  const resizeZone = useDevices((s) => s.resizeZone);
  const renameZone = useDevices((s) => s.renameZone);
  const identifyZone = useDevices((s) => s.identifyZone);

  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(zone.name);
  // Set when Escape cancels, so the trailing blur doesn't commit the edit.
  const cancelled = useRef(false);

  const step = (delta: number) => {
    const next = Math.min(
      zone.max_leds,
      Math.max(zone.min_leds, zone.led_count + delta),
    );
    if (next !== zone.led_count) resizeZone(deviceId, zone.id, next);
  };

  const startEdit = () => {
    setDraft(zone.name);
    cancelled.current = false;
    setEditing(true);
  };

  const commit = () => {
    setEditing(false);
    if (cancelled.current) return;
    const name = draft.trim();
    if (name !== zone.name) renameZone(deviceId, zone.id, name);
  };

  return (
    <div className="space-y-2">
      <div className="flex items-center gap-3">
        {editing ? (
          <input
            autoFocus
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") commit();
              if (e.key === "Escape") {
                cancelled.current = true;
                setEditing(false);
              }
            }}
            className="w-44 rounded bg-panel-2 px-2 py-0.5 text-sm font-semibold text-zinc-100 outline-none ring-1 ring-zinc-600 focus:ring-violet-500"
          />
        ) : (
          <button
            onClick={startEdit}
            title="Rename header"
            className="group flex items-center gap-1 text-sm font-semibold text-zinc-300 hover:text-zinc-100"
          >
            {zone.name}
            <span className="text-xs text-zinc-600 opacity-0 transition-opacity group-hover:opacity-100">
              ✎
            </span>
          </button>
        )}
        <span className="text-xs text-zinc-500">{zone.led_count} LEDs</span>
        <button
          onClick={() => identifyZone(deviceId, zone.id)}
          className="rounded bg-panel-2 px-2 py-0.5 text-xs text-zinc-300 hover:bg-zinc-700"
          title="Pulse this strip to identify which header it is"
        >
          ◉ Pulse
        </button>
        {zone.resizable && (
          <div className="flex items-center gap-1">
            <button
              onClick={() => step(-1)}
              className="h-6 w-6 rounded bg-panel-2 text-sm leading-none hover:bg-zinc-700"
              title="Fewer LEDs"
            >
              −
            </button>
            <button
              onClick={() => step(1)}
              className="h-6 w-6 rounded bg-panel-2 text-sm leading-none hover:bg-zinc-700"
              title="More LEDs"
            >
              +
            </button>
          </div>
        )}
      </div>
      <div className="flex flex-wrap gap-1.5 rounded-xl bg-black/40 p-3">
        {Array.from({ length: zone.led_count }, (_, i) => {
          const c = colors?.[i];
          const css = c ? cssColor(c) : "#27272a";
          return (
            <button
              key={i}
              onMouseDown={() => paintLed(deviceId, zone.id, i)}
              onMouseEnter={(e) => {
                if (e.buttons === 1) paintLed(deviceId, zone.id, i);
              }}
              className="h-4 w-4 rounded-full border border-black/40 transition-colors duration-75"
              style={{
                background: css,
                boxShadow: c ? `0 0 6px ${css}88` : undefined,
              }}
              title={`LED ${i + 1}`}
            />
          );
        })}
      </div>
    </div>
  );
}
