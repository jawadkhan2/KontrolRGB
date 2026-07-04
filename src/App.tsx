import { useEffect, useState } from "react";
import { DevicePage } from "./components/DevicePage";
import { EffectsLibraryPage } from "./components/EffectsLibraryPage";
import { SyncPage } from "./components/SyncPage";
import { FanPage } from "./components/FanPage";
import { SettingsPage } from "./components/SettingsPage";
import { Sidebar } from "./components/Sidebar";
import { TitleBar } from "./components/TitleBar";
import { ConflictModal } from "./components/ConflictModal";
import { BurstDebugModal } from "./components/BurstDebugModal";
import { startEventListeners } from "./lib/events";
import {
  scanRgbConflicts,
  isElevated,
  relaunchAsAdmin,
  type ConflictProcess,
} from "./lib/api";
import { fanStatus } from "./lib/fanApi";
import { useDevices } from "./store/devices";
import { useFans } from "./store/fans";
import { useSettings } from "./store/settings";

export type View = "sync" | "devices" | "library" | "fans" | "settings";

/**
 * Display refresh cadence for live fan readings. This only updates the UI —
 * actual fan control runs in the Rust backend's control loop, so this throttling
 * to ~1/min when the window is hidden no longer affects fan behaviour.
 */
const FAN_POLL_MS = 400;

const TYPE_SEG: Record<string, string> = {
  keyboard: "var(--kb)",
  motherboard: "var(--mb)",
  gpu: "var(--gpu)",
};

export default function App() {
  const init = useDevices((s) => s.init);
  const devices = useDevices((s) => s.devices);
  const selectedId = useDevices((s) => s.selectedId);
  const [view, setView] = useState<View>("sync");
  // Effects Library apply target: "all" or a device id. The sidebar opens it on
  // "all"; a device page's "Browse all" opens it preselected to that device.
  const [libraryTarget, setLibraryTarget] = useState<string>("all");
  const [conflicts, setConflicts] = useState<ConflictProcess[] | null>(null);

  const openLibrary = (target: string) => {
    setLibraryTarget(target);
    setView("library");
  };

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

  // If "ask to run as administrator on startup" is on and we're not elevated,
  // relaunch as admin (pops UAC). On success this process exits and the elevated
  // one takes over; if the user declines UAC we stay running unprivileged.
  useEffect(() => {
    if (!useSettings.getState().askAdminOnStartup) return;
    void (async () => {
      try {
        if (await isElevated()) return;
        await relaunchAsAdmin();
      } catch {
        /* UAC declined or not on Windows — carry on unprivileged */
      }
    })();
  }, []);

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

  // App-wide display refresh. Pulls live status/RPM/temps for whatever page is
  // open. Fan *control* no longer lives here — it runs in the backend control
  // loop (immune to webview timer throttling); this only updates the UI.
  useEffect(() => {
    const run = () => void useFans.getState().refresh();
    run();
    const t = window.setInterval(run, FAN_POLL_MS);
    return () => clearInterval(t);
  }, []);

  // Hand the saved control plan to the backend once the chip is up, so the
  // control loop engages (or idles, per the "control on startup" setting) even
  // if burst auto-detect is disabled. Burst also re-pushes after it maps fans.
  useEffect(() => {
    let done = false;
    const id = window.setInterval(async () => {
      if (done) return;
      try {
        const st = await fanStatus();
        if (!st.available) return;
        done = true;
        clearInterval(id);
        useFans.getState().pushControlPlan();
      } catch {
        /* chip not ready yet — retry next tick */
      }
    }, 1500);
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
        <Sidebar view={view} onChangeView={setView} onOpenLibrary={() => openLibrary("all")} />
        {view === "sync" ? (
          <SyncPage onChangeView={setView} />
        ) : view === "library" ? (
          <EffectsLibraryPage target={libraryTarget} onChangeTarget={setLibraryTarget} />
        ) : view === "fans" ? (
          <FanPage />
        ) : view === "settings" ? (
          <SettingsPage />
        ) : (
          <DevicePage onOpenLibrary={openLibrary} />
        )}
      </div>
    </div>
  );
}
