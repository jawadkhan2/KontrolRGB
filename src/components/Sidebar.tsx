import { useEffect, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import type { View } from "../App";
import { useDevices } from "../store/devices";
import { useFans, fansFullyControlled } from "../store/fans";
import type { DeviceInfo, DeviceType } from "../types/device";
import { cssColor } from "../types/device";

/* line-style nav icons (inherit currentColor → tinted by --ac) */
const TYPE_ICONS: Record<DeviceType, ReactNode> = {
  keyboard: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round">
      <rect x="2" y="6" width="20" height="12" rx="2.5" />
      <path d="M6 9.5h0M9.5 9.5h0M13 9.5h0M16.5 9.5h0M6 13h0M16.5 13h0" strokeWidth="2.2" />
      <path d="M9 13h6" />
    </svg>
  ),
  motherboard: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <rect x="3.5" y="3.5" width="17" height="17" rx="2" />
      <rect x="9" y="9" width="6" height="6" rx="1" />
      <path d="M9 3.5v-1M15 3.5v-1M9 21.5v1M15 21.5v1M3.5 9h-1M3.5 15h-1M21.5 9h1M21.5 15h1" />
    </svg>
  ),
  gpu: (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
      <rect x="2" y="6" width="20" height="12" rx="2" />
      <circle cx="8.5" cy="12" r="2.6" />
      <circle cx="15.5" cy="12" r="2.6" />
      <path d="M8.5 12h0M15.5 12h0" strokeWidth="2" />
    </svg>
  ),
};

const TYPE_ACCENT: Record<DeviceType, string> = {
  keyboard: "var(--kb)",
  motherboard: "var(--mb)",
  gpu: "var(--gpu)",
};

/* plain-English row labels; the exact model lives in the tooltip and the
   device page header. "Case Fans" because the board's ARGB header is what
   actually lights the fans. */
const TYPE_LABEL: Record<DeviceType, string> = {
  keyboard: "Keyboard",
  motherboard: "Case Fans",
  gpu: "GPU",
};

const LibraryIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
    <rect x="3" y="3" width="7.5" height="7.5" rx="1.6" />
    <rect x="13.5" y="3" width="7.5" height="7.5" rx="1.6" />
    <rect x="3" y="13.5" width="7.5" height="7.5" rx="1.6" />
    <path d="M17.25 13.5v7.5M13.5 17.25H21" />
  </svg>
);

const SyncIcon = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
    <path d="M21 2v6h-6M3 12a9 9 0 0 1 15-6.7L21 8M3 22v-6h6M21 12a9 9 0 0 1-15 6.7L3 16" />
  </svg>
);

const FanIcon = (
  <svg viewBox="0 0 24 24" fill="currentColor">
    <circle cx="12" cy="12" r="1.9" />
    <path d="M12 10.3c-.5-2.7.2-5.2 1.7-6.6 1.4 1 2 3 1.5 4.9-.4 1.4-1.7 1.9-3.2 1.7z" />
    <path d="M13.7 12c2.7-.5 5.2.2 6.6 1.7-1 1.4-3 2-4.9 1.5-1.4-.4-1.9-1.7-1.7-3.2z" />
    <path d="M12 13.7c.5 2.7-.2 5.2-1.7 6.6-1.4-1-2-3-1.5-4.9.4-1.4 1.7-1.9 3.2-1.7z" />
    <path d="M10.3 12c-2.7.5-5.2-.2-6.6-1.7 1-1.4 3-2 4.9-1.5 1.4.4 1.9 1.7 1.7 3.2z" />
  </svg>
);

const GearIcon = (
  <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
    <circle cx="12" cy="12" r="3" />
    <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
  </svg>
);

function LiveSwatch({ device }: { device: DeviceInfo }) {
  const frame = useDevices((s) => s.frames[device.id]);
  const firstZone = device.zones[0];
  const colors = frame?.[firstZone?.id] ?? [];
  // Sample a few LEDs across the zone for a mini gradient.
  const samples = [0, 0.25, 0.5, 0.75, 1].map((p) => {
    const c = colors[Math.floor(p * Math.max(0, colors.length - 1))];
    return c ? cssColor(c) : "#27272a";
  });
  return (
    <span
      className="swatch"
      style={{ background: `linear-gradient(90deg, ${samples.join(", ")})`, animation: "none" }}
    />
  );
}

export function Sidebar({
  view,
  onChangeView,
  onOpenLibrary,
}: {
  view: View;
  onChangeView: (v: View) => void;
  onOpenLibrary: () => void;
}) {
  const devices = useDevices((s) => s.devices);
  const selectedId = useDevices((s) => s.selectedId);
  const select = useDevices((s) => s.select);
  // Green only when every controllable fan (pump excluded) is on the app, not the
  // BIOS. Any fan still under BIOS control → dim dot.
  const fansControlled = useFans(fansFullyControlled);
  const [version, setVersion] = useState("");
  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  return (
    <aside className="sidebar">
      {/* hero card — the sync action and glanceable system status share one surface */}
      <div className="head">
        <button
          className={`sync-card ${view === "sync" ? "active" : ""}`}
          onClick={() => onChangeView("sync")}
        >
          <span className="sync-main">
            <span className="ico" style={{ ["--ac" as string]: "var(--accent)" }}>{SyncIcon}</span>
            <span className="meta">
              <span className="name">Sync All</span>
            </span>
          </span>
          <span className="sync-status">
            <span className="online" />
            <span className="line">
              <b>All synced</b>
              <span className="sep">·</span>
              {devices.length} lighting
              <span className="sep">·</span>
              case fans
            </span>
          </span>
        </button>
      </div>

      <nav className="nav">
        <button
          onClick={onOpenLibrary}
          className={`nav-item ${view === "library" ? "active" : ""}`}
          style={{ ["--ac" as string]: "var(--accent)" }}
        >
          <span className="ico">{LibraryIcon}</span>
          <span className="meta">
            <span className="name">Effects Library</span>
            <span className="sub">Browse every effect</span>
          </span>
        </button>
      </nav>

      <div className="sec-label">
        <span>Lighting</span>
        <span className="count">{devices.length}</span>
      </div>
      <nav className="nav">
        {devices.map((d) => {
          const active = d.id === selectedId && view === "devices";
          return (
            <button
              key={d.id}
              onClick={() => {
                select(d.id);
                onChangeView("devices");
              }}
              className={`nav-item ${active ? "active" : ""}`}
              style={{ ["--ac" as string]: TYPE_ACCENT[d.device_type] }}
              title={d.name}
            >
              <span className="ico">{TYPE_ICONS[d.device_type]}</span>
              <span className="meta">
                <span className="name">{TYPE_LABEL[d.device_type]}</span>
                <LiveSwatch device={d} />
              </span>
              <span className="dot" />
            </button>
          );
        })}
      </nav>

      <div className="sec-label">
        <span>Cooling</span>
      </div>
      <nav className="nav">
        <button
          onClick={() => onChangeView("fans")}
          className={`nav-item ${view === "fans" ? "active" : ""}`}
          style={{ ["--ac" as string]: "var(--fan)" }}
        >
          <span className="ico">{FanIcon}</span>
          <span className="meta">
            <span className="name">Fan Control</span>
            <span className="sub">Case fans · monitor</span>
          </span>
          <span className={`dot ${fansControlled ? "" : "off"}`}
            title={fansControlled ? "All fans under app control" : "Some fans on the BIOS curve"} />
        </button>
      </nav>

      <div className="foot">
        <button
          className={`nav-item ${view === "settings" ? "active" : ""}`}
          onClick={() => onChangeView("settings")}
          style={{ ["--ac" as string]: "var(--dim)" }}
        >
          <span className="ico gear-rot">{GearIcon}</span>
          <span className="meta">
            <span className="name">Settings</span>
          </span>
        </button>
        {version && <span className="ver">v{version}</span>}
      </div>
    </aside>
  );
}
