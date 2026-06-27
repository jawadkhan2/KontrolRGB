import { useEffect, useState } from "react";
import { DevicePage } from "./components/DevicePage";
import { FanPage } from "./components/FanPage";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { ConflictModal } from "./components/ConflictModal";
import { BurstDebugModal } from "./components/BurstDebugModal";
import { startEventListeners } from "./lib/events";
import { scanRgbConflicts, type ConflictProcess } from "./lib/api";
import { fanStatus } from "./lib/fanApi";
import { useDevices } from "./store/devices";
import { useFans } from "./store/fans";
import { useSettings } from "./store/settings";

export type View = "devices" | "fans" | "settings";

const TYPE_SEG: Record<string, string> = {
  keyboard: "var(--kb)",
  motherboard: "var(--mb)",
  gpu: "var(--gpu)",
};

export default function App() {
  const init = useDevices((s) => s.init);
  const devices = useDevices((s) => s.devices);
  const selectedId = useDevices((s) => s.selectedId);
  const [view, setView] = useState<View>("devices");
  const [conflicts, setConflicts] = useState<ConflictProcess[] | null>(null);

  // Active section accent → drives the page glow + shared control colors.
  const selectedType = devices.find((d) => d.id === selectedId)?.device_type;
  const seg =
    view === "fans"
      ? "var(--fan)"
      : view === "devices" && selectedType
      ? TYPE_SEG[selectedType]
      : "var(--accent)";

  useEffect(() => {
    const stop = startEventListeners();
    void init();
    scanRgbConflicts().then(setConflicts).catch(() => setConflicts([]));
    return stop;
  }, [init]);

  // Startup fan auto-detect, app-wide (independent of which page is open). Once
  // the chip is available and the RGB/fan conflict guard has been cleared, fire
  // one burst run. The backend refuses until conflicts are killed, so this
  // simply retries each tick until it succeeds, then stops.
  useEffect(() => {
    let id = 0;
    const tick = async () => {
      const f = useFans.getState();
      if (f.burstDone) {
        clearInterval(id);
        return;
      }
      if (!useSettings.getState().burstOnStartup || f.burstDetecting) return;
      try {
        const st = await fanStatus();
        if (st.available) await f.runBurstDetect();
      } catch {
        /* not available yet / transient — retry next tick */
      }
    };
    id = window.setInterval(() => void tick(), 1500);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="flex h-full flex-col" style={{ ["--seg" as string]: seg }}>
      {conflicts && conflicts.length > 0 && (
        <ConflictModal processes={conflicts} onDone={() => setConflicts([])} />
      )}
      <BurstDebugModal />
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar view={view} onChangeView={setView} />
        {view === "fans" ? (
          <FanPage />
        ) : view === "settings" ? (
          <SettingsPage />
        ) : (
          <DevicePage />
        )}
      </div>
    </div>
  );
}
