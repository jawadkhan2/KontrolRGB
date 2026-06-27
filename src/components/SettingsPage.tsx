import { useSettings } from "../store/settings";

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
        className={`relative h-6 w-11 shrink-0 rounded-full transition-colors ${
          checked ? "bg-accent" : "bg-panel-2"
        }`}
      >
        <span
          className={`absolute top-0.5 h-5 w-5 rounded-full bg-white transition-transform ${
            checked ? "translate-x-5" : "translate-x-0.5"
          }`}
        />
      </button>
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

export function SettingsPage() {
  const burstOnStartup = useSettings((s) => s.burstOnStartup);
  const setBurstOnStartup = useSettings((s) => s.setBurstOnStartup);
  const burstDebug = useSettings((s) => s.burstDebug);
  const setBurstDebug = useSettings((s) => s.setBurstDebug);
  const burstSeconds = useSettings((s) => s.burstSeconds);
  const setBurstSeconds = useSettings((s) => s.setBurstSeconds);

  return (
    <main className="flex-1 overflow-y-auto">
      <div className="mx-auto max-w-2xl space-y-5 p-6">
        <header>
          <h2 className="text-xl font-bold">Settings</h2>
          <p className="mt-0.5 text-sm text-zinc-500">App-wide preferences</p>
        </header>

        <SettingsSection title="Fan control">
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
        </SettingsSection>
      </div>
    </main>
  );
}
