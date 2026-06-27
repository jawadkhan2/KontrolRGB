import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

const appWindow = getCurrentWindow();

/** App logomark: a control-dial / aperture of 6 RGB nodes around a hub. */
function LogoMark() {
  return (
    <svg viewBox="0 0 32 32" aria-hidden="true">
      <g className="ring">
        <circle cx="16" cy="5.5" r="2.4" fill="#ff5a4d" />
        <circle cx="25.1" cy="10.75" r="2.4" fill="#fbbf24" />
        <circle cx="25.1" cy="21.25" r="2.4" fill="#84cc16" />
        <circle cx="16" cy="26.5" r="2.4" fill="#22d3ee" />
        <circle cx="6.9" cy="21.25" r="2.4" fill="#5b8cff" />
        <circle cx="6.9" cy="10.75" r="2.4" fill="#a855f7" />
      </g>
      <circle cx="16" cy="16" r="5.4" fill="none" stroke="#3a3f4a" strokeWidth="1.4" />
      <circle cx="16" cy="16" r="2.2" fill="#e7e9ee" />
    </svg>
  );
}

export function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    appWindow.isMaximized().then(setMaximized).catch(() => {});
    const unlisten = appWindow.onResized(() => {
      appWindow.isMaximized().then(setMaximized).catch(() => {});
    });
    return () => { unlisten.then((f) => f()).catch(() => {}); };
  }, []);

  function onDragStart(e: React.MouseEvent) {
    if (e.button === 0) void appWindow.startDragging();
  }

  return (
    <div className="titlebar">
      {/* animated spectrum stripe (doubles as a drag handle) */}
      <div className="stripe" onMouseDown={onDragStart} />

      <div className="bar" onMouseDown={onDragStart}>
        <span className="brandmark pointer-events-none">
          <LogoMark />
        </span>
        <span className="wordmark pointer-events-none">
          KONTROL<b>RGB</b>
        </span>

        {/* window controls — stop mousedown so they don't trigger drag */}
        <div className="win-ctrls" onMouseDown={(e) => e.stopPropagation()}>
          <button onClick={() => appWindow.minimize()} aria-label="Minimize">
            <svg width="10" height="1" viewBox="0 0 10 1" fill="currentColor">
              <rect width="10" height="1" rx="0.5" />
            </svg>
          </button>
          <button
            onClick={() => appWindow.toggleMaximize()}
            aria-label={maximized ? "Restore" : "Maximize"}
          >
            {maximized ? (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2">
                <rect x="2.5" y="0.5" width="7" height="7" rx="0.8" />
                <path d="M0.5 2.5v7h7" />
              </svg>
            ) : (
              <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.2">
                <rect x="0.5" y="0.5" width="9" height="9" rx="0.8" />
              </svg>
            )}
          </button>
          <button className="close" onClick={() => appWindow.close()} aria-label="Close">
            <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <line x1="1" y1="1" x2="9" y2="9" />
              <line x1="9" y1="1" x2="1" y2="9" />
            </svg>
          </button>
        </div>
      </div>
    </div>
  );
}
