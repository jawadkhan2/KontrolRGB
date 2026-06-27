import { useEffect, useState } from "react";
import type { BurstFanProgress } from "../lib/fanApi";
import { useFans } from "../store/fans";
import { useSettings } from "../store/settings";

// Diagnostic overlay shown while the startup burst auto-detect runs. Streams the
// live per-fan RPM emitted by the backend (`fan-burst-progress`) so you can
// watch each header spin up over the burst window. Whatever is spinning at the
// end of the window gets mapped as a controllable fan. Gated by the `burstDebug`
// setting; mounted app-wide from App.tsx.

function StatusBadge({ fan }: { fan: BurstFanProgress }) {
  const cls = fan.detected
    ? "bg-emerald-500/15 text-emerald-400"
    : "text-zinc-500 bg-panel-2";
  return (
    <span className={`rounded px-2 py-0.5 text-xs font-medium ${cls}`}>
      {fan.detected ? "spinning" : "no fan"}
    </span>
  );
}

function FanRow({ fan }: { fan: BurstFanProgress }) {
  return (
    <tr className="border-t border-panel-2">
      <td className="py-2 pr-3">
        <div className="text-sm font-medium text-zinc-200">
          {fan.headerLabel}
        </div>
        <div className="text-xs text-zinc-500">
          hdr {fan.header} · tach {fan.rpmChannel}
        </div>
      </td>
      <td className="py-2 px-3 text-right font-mono text-sm tabular-nums text-zinc-100">
        {fan.rpm}
      </td>
      <td className="py-2 px-3 text-right font-mono text-sm tabular-nums text-zinc-400">
        {fan.maxRpm}
      </td>
      <td className="py-2 pl-3 text-right">
        <StatusBadge fan={fan} />
      </td>
    </tr>
  );
}

export function BurstDebugModal() {
  const enabled = useSettings((s) => s.burstDebug);
  const progress = useFans((s) => s.burstProgress);
  const detecting = useFans((s) => s.burstDetecting);
  const [dismissed, setDismissed] = useState(false);

  // A fresh run un-dismisses so the modal reappears each detect.
  useEffect(() => {
    if (detecting) setDismissed(false);
  }, [detecting]);

  if (!enabled || !progress || dismissed) return null;

  const done = progress.phase === "done" && !detecting;
  const detected = progress.fans.filter((f) => f.detected).length;
  const elapsed = (progress.elapsedMs / 1000).toFixed(1);
  const total = Math.round(progress.totalMs / 1000);
  const pct = progress.totalMs
    ? Math.min(100, (progress.elapsedMs / progress.totalMs) * 100)
    : 0;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 p-6">
      <div className="w-full max-w-xl rounded-xl border border-panel-2 bg-panel shadow-2xl">
        <div className="flex items-center justify-between border-b border-panel-2 px-5 py-4">
          <div>
            <h2 className="text-base font-bold">Fan auto-detect</h2>
            <p className="mt-0.5 text-xs text-zinc-500">
              {done
                ? `Done — ${detected} fan${detected === 1 ? "" : "s"} mapped`
                : `Bursting all headers to 100% · ${elapsed}s / ${total}s`}
            </p>
          </div>
          <span
            className={`flex items-center gap-2 text-xs font-medium ${
              done ? "text-emerald-400" : "text-amber-400"
            }`}
          >
            {!done && (
              <span className="h-2 w-2 animate-pulse rounded-full bg-amber-400" />
            )}
            {done ? "done" : "bursting"}
          </span>
        </div>

        {!done && (
          <div className="h-1 w-full bg-panel-2">
            <div
              className="h-full bg-amber-400 transition-all duration-300"
              style={{ width: `${pct}%` }}
            />
          </div>
        )}

        <div className="px-5 py-2">
          <table className="w-full">
            <thead>
              <tr className="text-xs uppercase tracking-wider text-zinc-500">
                <th className="py-2 pr-3 text-left font-medium">Header</th>
                <th className="py-2 px-3 text-right font-medium">RPM</th>
                <th className="py-2 px-3 text-right font-medium">Max</th>
                <th className="py-2 pl-3 text-right font-medium">Status</th>
              </tr>
            </thead>
            <tbody>
              {progress.fans.map((f) => (
                <FanRow key={f.header} fan={f} />
              ))}
            </tbody>
          </table>
        </div>

        <div className="flex items-center justify-between border-t border-panel-2 px-5 py-3">
          <span className="text-xs text-zinc-500">
            Debug overlay — disable in Settings → Fan control.
          </span>
          <button
            onClick={() => setDismissed(true)}
            disabled={!done}
            className={`rounded-lg px-4 py-2 text-sm font-semibold transition-colors ${
              done
                ? "bg-accent/90 text-white hover:bg-accent"
                : "cursor-not-allowed bg-panel-2 text-zinc-500"
            }`}
          >
            {done ? "Close" : "Detecting…"}
          </button>
        </div>
      </div>
    </div>
  );
}
