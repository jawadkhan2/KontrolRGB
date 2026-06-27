import { useState } from "react";
import type { ConflictProcess } from "../lib/api";
import { killRgbConflicts, quitApp } from "../lib/api";

type Phase = "confirm" | "killing" | "done" | "error";

interface Props {
  processes: ConflictProcess[];
  onDone: () => void;
}

export function ConflictModal({ processes, onDone }: Props) {
  const [phase, setPhase] = useState<Phase>("confirm");
  const [error, setError] = useState<string | null>(null);

  async function handleKillAll() {
    setPhase("killing");
    try {
      await killRgbConflicts(processes.map((p) => p.pid));
      setPhase("done");
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
      setPhase("error");
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70">
      <div className="w-full max-w-md rounded-xl bg-panel border border-panel-2 p-6 shadow-2xl">
        {phase === "confirm" || phase === "killing" ? (
          <>
            <h2 className="text-lg font-semibold text-white mb-1">
              Conflicting RGB Software Detected
            </h2>
            <p className="text-sm text-zinc-400 mb-4">
              These apps are running and may fight KontrolRGB for hardware
              access. Killing them prevents wedged devices and unstable fan
              control.
            </p>
            <ul className="mb-5 space-y-2">
              {processes.map((p) => (
                <li
                  key={p.pid}
                  className="flex items-center gap-3 rounded-lg bg-panel-2 px-3 py-2"
                >
                  <div>
                    <div className="text-sm font-medium text-white">
                      {p.displayName}
                    </div>
                    <div className="text-xs text-zinc-500">
                      {p.exeName} &middot; PID {p.pid}
                    </div>
                  </div>
                </li>
              ))}
            </ul>
            <div className="flex gap-3 justify-end">
              <button
                onClick={onDone}
                disabled={phase === "killing"}
                className="rounded-md px-4 py-2 text-sm text-zinc-400 hover:text-zinc-200 disabled:opacity-40"
              >
                Skip & Continue
              </button>
              <button
                onClick={handleKillAll}
                disabled={phase === "killing"}
                className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-white hover:bg-accent/80 disabled:opacity-50"
              >
                {phase === "killing" ? "Killing…" : "Kill All & Continue"}
              </button>
            </div>
          </>
        ) : phase === "done" ? (
          <>
            <h2 className="text-lg font-semibold text-white mb-1">
              Processes Terminated
            </h2>
            <p className="text-sm text-zinc-400 mb-4">
              All conflicting software has been shut down.
            </p>
            <ul className="mb-5 space-y-2">
              {processes.map((p) => (
                <li
                  key={p.pid}
                  className="flex items-center gap-3 rounded-lg bg-panel-2 px-3 py-2"
                >
                  <span className="text-emerald-400 text-base leading-none">✓</span>
                  <div>
                    <div className="text-sm font-medium text-white">
                      {p.displayName}
                    </div>
                    <div className="text-xs text-zinc-500">
                      {p.exeName} &middot; PID {p.pid}
                    </div>
                  </div>
                </li>
              ))}
            </ul>
            <div className="flex justify-end">
              <button
                onClick={onDone}
                className="rounded-md bg-accent px-4 py-2 text-sm font-semibold text-white hover:bg-accent/80"
              >
                Close
              </button>
            </div>
          </>
        ) : (
          <>
            <h2 className="text-lg font-semibold text-white mb-1">
              Some Processes Could Not Be Killed
            </h2>
            <p className="text-sm text-zinc-400 mb-2">{error}</p>
            <div className="rounded-lg bg-amber-950/60 border border-amber-800/50 px-3 py-2 mb-3">
              <p className="text-xs text-amber-300 font-medium mb-0.5">
                Administrator privileges required
              </p>
              <p className="text-xs text-amber-400/80">
                KontrolRGB is not running as Administrator. Restart it with
                "Run as administrator" to force-close these processes.
                Continuing without killing them risks hardware conflicts.
              </p>
            </div>
            <p className="text-xs text-zinc-500 mb-5">
              Alternatively, close the conflicting apps manually before continuing.
            </p>
            <div className="flex gap-3 justify-end">
              <button
                onClick={() => quitApp()}
                className="rounded-md bg-red-900/60 border border-red-700/50 px-4 py-2 text-sm font-semibold text-red-300 hover:bg-red-800/60"
              >
                Quit App
              </button>
              <button
                onClick={onDone}
                className="rounded-md bg-panel-2 px-4 py-2 text-sm text-zinc-300 hover:bg-zinc-700"
              >
                Continue Anyway
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}
