import { useEffect, useState } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useSettings } from "../store/settings";
import { useFans } from "../store/fans";
import {
  getGuardConfig,
  openLogFolder,
  setGuardConfig,
  type GuardConfig,
} from "../lib/api";
import {
  checkForUpdate,
  installUpdate,
  type UpdateStatus,
} from "../lib/updater";

/** A labelled on/off row used across settings sections. */
function ToggleRow({
  title,
  description,
  checked,
  onChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-5 py-4">
      <div>
        <div className="text-sm font-medium text-zinc-200">{title}</div>
        <p className="mt-0.5 text-xs text-zinc-500">{description}</p>
      </div>
      <button
        role="switch"
        aria-checked={checked}
        onClick={() => onChange(!checked)}
        className={`toggle ${checked ? "on" : ""}`}
      />
    </div>
  );
}

/** A labelled numeric stepper row (bounded integer seconds, etc.). */
function NumberRow({
  title,
  description,
  value,
  min,
  max,
  unit,
  onChange,
}: {
  title: string;
  description: string;
  value: number;
  min: number;
  max: number;
  unit: string;
  onChange: (v: number) => void;
}) {
  const clamp = (v: number) => Math.max(min, Math.min(max, v));
  return (
    <div className="flex items-center justify-between gap-4 px-5 py-4">
      <div>
        <div className="text-sm font-medium text-zinc-200">{title}</div>
        <p className="mt-0.5 text-xs text-zinc-500">{description}</p>
      </div>
      <div className="flex shrink-0 items-center gap-2">
        <button
          onClick={() => onChange(clamp(value - 1))}
          disabled={value <= min}
          className="h-7 w-7 rounded-md bg-panel-2 text-sm font-bold text-zinc-300 transition-colors hover:text-white disabled:opacity-40"
        >
          −
        </button>
        <span className="w-14 text-center font-mono text-sm tabular-nums text-zinc-100">
          {value}
          {unit}
        </span>
        <button
          onClick={() => onChange(clamp(value + 1))}
          disabled={value >= max}
          className="h-7 w-7 rounded-md bg-panel-2 text-sm font-bold text-zinc-300 transition-colors hover:text-white disabled:opacity-40"
        >
          +
        </button>
      </div>
    </div>
  );
}

/** A labelled segmented-choice row (two or more mutually exclusive options). */
function SegRow<T extends string>({
  title,
  description,
  value,
  options,
  onChange,
}: {
  title: string;
  description: string;
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 px-5 py-4">
      <div>
        <div className="text-sm font-medium text-zinc-200">{title}</div>
        <p className="mt-0.5 text-xs text-zinc-500">{description}</p>
      </div>
      <div className="seg-ctrl shrink-0">
        {options.map((o) => (
          <button
            key={o.value}
            className={value === o.value ? "on" : ""}
            onClick={() => onChange(o.value)}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** A titled card grouping related settings. */
function SettingsSection({
  title,
  children,
}: {
  title: string;
  children: React.ReactNode;
}) {
  return (
    <section className="rounded-xl border border-panel-2 bg-panel">
      <div className="border-b border-panel-2 px-5 py-3">
        <span className="text-xs font-semibold uppercase tracking-wider text-zinc-500">
          {title}
        </span>
      </div>
      <div className="divide-y divide-panel-2">{children}</div>
    </section>
  );
}

/** Update-check row: shows the current version and a button that checks GitHub
 *  releases, then downloads/installs + restarts when a newer build is found. */
function UpdateRow() {
  const [version, setVersion] = useState("");
  const [status, setStatus] = useState<UpdateStatus>({ kind: "idle" });

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  const busy =
    status.kind === "checking" ||
    status.kind === "downloading" ||
    status.kind === "installing";

  const run = async () => {
    setStatus({ kind: "checking" });
    try {
      const update = await checkForUpdate();
      if (!update) {
        setStatus({ kind: "uptodate" });
        return;
      }
      // Update found — install immediately. installUpdate restarts on success,
      // so anything past this only runs on failure.
      setStatus({
        kind: "available",
        version: update.version,
        notes: update.body,
      });
      await installUpdate(update, setStatus);
    } catch (e) {
      setStatus({ kind: "error", message: String(e) });
    }
  };

  const statusText = (() => {
    switch (status.kind) {
      case "checking":
        return "Checking GitHub releases…";
      case "uptodate":
        return "You're on the latest version.";
      case "available":
        return `Update ${status.version} found — preparing…`;
      case "downloading":
        return `Downloading update… ${status.pct}%`;
      case "installing":
        return "Installing — the app will restart…";
      case "error":
        return `Update check failed: ${status.message}`;
      default:
        return "";
    }
  })();

  return (
    <div className="flex items-center justify-between gap-4 px-5 py-4">
      <div>
        <div className="text-sm font-medium text-zinc-200">
          Check for updates
        </div>
        <p className="mt-0.5 text-xs text-zinc-500">
          KontrolRGB {version && `v${version}`} · updates are downloaded from
          GitHub releases and installed over the air.
        </p>
        {statusText && (
          <p
            className={`mt-1 text-xs ${
              status.kind === "error" ? "text-red-400" : "text-zinc-400"
            }`}
          >
            {statusText}
          </p>
        )}
      </div>
      <button
        onClick={() => void run()}
        disabled={busy}
        className="shrink-0 rounded-lg bg-panel-2 px-4 py-2 text-sm font-semibold text-zinc-100 transition-colors hover:text-white disabled:cursor-not-allowed disabled:opacity-40"
      >
        {busy ? "Working…" : "Check now"}
      </button>
    </div>
  );
}

/** Launch-on-boot toggle. The Windows Run-key registration is the source of
 *  truth (not localStorage), so we read the live state from the autostart
 *  plugin on mount and write straight back to it on toggle. */
function StartupRow() {
  const [enabled, setEnabled] = useState(false);
  const [ready, setReady] = useState(false);

  useEffect(() => {
    isEnabled()
      .then(setEnabled)
      .catch(() => {})
      .finally(() => setReady(true));
  }, []);

  const onChange = async (v: boolean) => {
    // Optimistic flip; revert if the registry write fails.
    setEnabled(v);
    try {
      if (v) await enable();
      else await disable();
    } catch {
      setEnabled(!v);
    }
  };

  return (
    <ToggleRow
      title="Start with Windows"
      description="Launch KontrolRGB automatically when you sign in, hidden in the system tray. Effects and fan control resume in the background without opening the window."
      checked={ready && enabled}
      onChange={(v) => void onChange(v)}
    />
  );
}

/** Backend conflict guard (auto-close other RGB/fan software) + log access. The
 *  settings live in the backend config, not localStorage, because the guard
 *  runs before the window ever opens on a boot launch. */
function GuardRows() {
  const [cfg, setCfg] = useState<GuardConfig | null>(null);

  useEffect(() => {
    getGuardConfig().then(setCfg).catch(() => {});
  }, []);

  const update = (patch: Partial<GuardConfig>) => {
    if (!cfg) return;
    const next = { ...cfg, ...patch };
    setCfg(next);
    setGuardConfig(next).catch(() => setCfg(cfg));
  };

  return (
    <>
      <ToggleRow
        title="Keep other RGB / fan software closed"
        description="KontrolRGB is the only app driving your lighting and fans. Checks every 15 seconds and shuts down anything that competes for the hardware (MSI Center & Mystic Light services, LED Keeper, SignalRGB, OpenRGB, iCUE, Armoury Crate, …), then re-applies your lighting. Needs administrator rights to stop services."
        checked={!!cfg?.autoEnforce}
        onChange={(v) => update({ autoEnforce: v })}
      />
      <ToggleRow
        title="Disable their Windows services"
        description="When a conflicting app runs as a Windows service, also set the service to Disabled so it doesn't come back after a reboot. Re-enable it from services.msc if you ever want it back."
        checked={!!cfg?.disableServices}
        onChange={(v) => update({ disableServices: v })}
      />
      <div className="flex items-center justify-between gap-4 px-5 py-4">
        <div>
          <div className="text-sm font-medium text-zinc-200">Diagnostic log</div>
          <p className="mt-0.5 text-xs text-zinc-500">
            KontrolRGB logs hardware recovery, wake events and conflicts to
            kontrolrgb.log. Attach it when reporting a problem.
          </p>
        </div>
        <button
          onClick={() => void openLogFolder()}
          className="shrink-0 rounded-lg bg-panel-2 px-4 py-2 text-sm font-semibold text-zinc-100 transition-colors hover:text-white"
        >
          Open folder
        </button>
      </div>
    </>
  );
}

export function SettingsPage() {
  const fanControlOnStartup = useSettings((s) => s.fanControlOnStartup);
  const setFanControlOnStartup = useSettings((s) => s.setFanControlOnStartup);
  const burstOnStartup = useSettings((s) => s.burstOnStartup);
  const setBurstOnStartup = useSettings((s) => s.setBurstOnStartup);
  const burstDebug = useSettings((s) => s.burstDebug);
  const setBurstDebug = useSettings((s) => s.setBurstDebug);
  const burstSeconds = useSettings((s) => s.burstSeconds);
  const setBurstSeconds = useSettings((s) => s.setBurstSeconds);
  const syncFallbackMode = useSettings((s) => s.syncFallbackMode);
  const setSyncFallbackMode = useSettings((s) => s.setSyncFallbackMode);
  const askAdminOnStartup = useSettings((s) => s.askAdminOnStartup);
  const setAskAdminOnStartup = useSettings((s) => s.setAskAdminOnStartup);

  const stop = useFans((s) => s.stop);
  const stopping = useFans((s) => s.stopping);
  const fanStatus = useFans((s) => s.status);
  const manualActive = !!fanStatus?.manualActive;
  const fanAvailable = !!fanStatus?.available;

  return (
    <main className="flex-1 overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-5 p-6">
        <header>
          <h2 className="text-xl font-bold">Settings</h2>
          <p className="mt-0.5 text-sm text-zinc-500">App-wide preferences</p>
        </header>

        <SettingsSection title="Application">
          <StartupRow />
          <ToggleRow
            title="Ask to run as administrator on startup"
            description="On launch, if KontrolRGB isn't already elevated, prompt for administrator rights (Windows UAC). Recommended — direct hardware paths like the ring-0 fan driver need admin. Decline the prompt and the app keeps running unprivileged."
            checked={askAdminOnStartup}
            onChange={setAskAdminOnStartup}
          />
          <UpdateRow />
        </SettingsSection>

        <SettingsSection title="Hardware ownership">
          <GuardRows />
        </SettingsSection>

        <SettingsSection title="Lighting">
          <SegRow
            title="Limited devices"
            description="Some devices can't host-animate every effect (the GMMK keyboard runs animations in its own firmware). When you sync an effect they can't host, run the closest onboard firmware effect, or leave those devices out of the sync."
            value={syncFallbackMode}
            options={[
              { value: "fallback", label: "Closest match" },
              { value: "exclude", label: "Skip them" },
            ]}
            onChange={setSyncFallbackMode}
          />
        </SettingsSection>

        <SettingsSection title="Fan control">
          <ToggleRow
            title="Control fans on startup"
            description="Engage background fan control as soon as the app starts: mapped fans are taken off the BIOS and onto their saved profiles, held by the backend even when the window is minimized. Turn off to leave fans on the BIOS curve until you change a fan's mode or speed."
            checked={fanControlOnStartup}
            onChange={setFanControlOnStartup}
          />
          <ToggleRow
            title="Auto-detect fans on startup"
            description="After conflicting apps are closed, briefly spin every header to 100% and auto-map the ones with a fan attached. The pump is left under BIOS control. Runs in the background."
            checked={burstOnStartup}
            onChange={setBurstOnStartup}
          />
          <NumberRow
            title="Burst duration"
            description="How long to hold every header at 100% before snapshotting which fans are spinning. Longer catches slow movers (SYS_FAN 5/6); shorter is quieter."
            value={burstSeconds}
            min={2}
            max={30}
            unit="s"
            onChange={setBurstSeconds}
          />
          <ToggleRow
            title="Show burst debug modal"
            description="While startup auto-detect runs, pop a modal showing live per-fan RPM, acceleration, and plateau state. Diagnostic only — turn off for a silent background detect."
            checked={burstDebug}
            onChange={setBurstDebug}
          />
          <div className="flex items-center justify-between gap-4 px-5 py-4">
            <div>
              <div className="text-sm font-medium text-zinc-200">Release fans to BIOS</div>
              <p className="mt-0.5 text-xs text-zinc-500">
                Hand every fan back to the motherboard's BIOS curve and drop KontrolRGB's
                manual control. Fans re-engage when you change a profile, mode, or speed.
              </p>
            </div>
            <button
              onClick={() => void stop()}
              disabled={!fanAvailable || stopping || !manualActive}
              title={manualActive ? "Release all fans back to BIOS" : "No fans under manual control"}
              className="shrink-0 rounded-lg bg-red-600 px-4 py-2 text-sm font-semibold text-white shadow-lg shadow-red-900/40 transition-colors hover:bg-red-500 disabled:cursor-not-allowed disabled:opacity-40 disabled:shadow-none"
            >
              {stopping ? "Releasing…" : "■ STOP → BIOS"}
            </button>
          </div>
        </SettingsSection>
      </div>
    </main>
  );
}
