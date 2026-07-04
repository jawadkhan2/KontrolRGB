import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import * as fanApi from "../lib/fanApi";
import { useSettings } from "./settings";
import type {
  BurstProgress,
  ChannelReading,
  ControlMode,
  FanControlPlan,
  FanStatus,
  SweepProgress,
  SweepResult,
  TempReading,
} from "../lib/fanApi";

// --- Types ---

export type TempSourceKey = "cpu" | "aux0" | "aux1" | "aux2" | "aux3" | "gpu";

export interface CurvePoint {
  tempC: number;
  speedPct: number;
}

export interface FanProfile {
  id: string;
  name: string;
  isBuiltin: boolean;
  tempSource: TempSourceKey;
  points: CurvePoint[];
}

export type FanMode =
  | { type: "curve"; profileId: string }
  | { type: "manual"; pct: number };

// --- Built-in profiles ---

export const BUILTIN_PROFILES: FanProfile[] = [
  {
    id: "silent",
    name: "Silent",
    isBuiltin: true,
    tempSource: "cpu",
    points: [
      { tempC: 20, speedPct: 20 },
      { tempC: 50, speedPct: 25 },
      { tempC: 65, speedPct: 40 },
      { tempC: 75, speedPct: 60 },
      { tempC: 85, speedPct: 85 },
      { tempC: 95, speedPct: 100 },
    ],
  },
  {
    id: "balanced",
    name: "Balanced",
    isBuiltin: true,
    tempSource: "cpu",
    points: [
      { tempC: 20, speedPct: 30 },
      { tempC: 50, speedPct: 40 },
      { tempC: 65, speedPct: 60 },
      { tempC: 75, speedPct: 80 },
      { tempC: 85, speedPct: 95 },
      { tempC: 95, speedPct: 100 },
    ],
  },
  {
    id: "performance",
    name: "Performance",
    isBuiltin: true,
    tempSource: "cpu",
    points: [
      { tempC: 20, speedPct: 50 },
      { tempC: 50, speedPct: 65 },
      { tempC: 65, speedPct: 80 },
      { tempC: 75, speedPct: 90 },
      { tempC: 85, speedPct: 100 },
    ],
  },
  {
    id: "max",
    name: "Max",
    isBuiltin: true,
    tempSource: "cpu",
    points: [{ tempC: 20, speedPct: 100 }],
  },
];

// --- Curve interpolation ---

export function interpolateCurve(points: CurvePoint[], tempC: number): number {
  if (points.length === 0) return 50;
  const sorted = [...points].sort((a, b) => a.tempC - b.tempC);
  if (tempC <= sorted[0].tempC) return sorted[0].speedPct;
  if (tempC >= sorted[sorted.length - 1].tempC) return sorted[sorted.length - 1].speedPct;
  for (let i = 0; i < sorted.length - 1; i++) {
    const a = sorted[i], b = sorted[i + 1];
    if (tempC >= a.tempC && tempC <= b.tempC) {
      const t = (tempC - a.tempC) / (b.tempC - a.tempC);
      return Math.round(a.speedPct + t * (b.speedPct - a.speedPct));
    }
  }
  return sorted[sorted.length - 1].speedPct;
}

// --- Persistence ---

const STORAGE_KEY = "kontrolrgb-fan-state-v1";

interface PersistedState {
  customProfiles: FanProfile[];
  fanModes: Record<number, FanMode>;
  activeProfileId: string;
  /** User-renamed fans, keyed by tach channel. */
  fanNames: Record<number, string>;
}

function loadSaved(): Partial<PersistedState> {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    return s ? (JSON.parse(s) as Partial<PersistedState>) : {};
  } catch {
    return {};
  }
}

function savePersisted(state: PersistedState) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {}
}

// --- Store ---

interface FansStore {
  // Hardware state
  status: FanStatus | null;
  readings: ChannelReading[];
  temps: TempReading[];
  loaded: boolean;
  stopping: boolean;
  /**
   * True while fans should be left on the BIOS curve — either the user pressed
   * STOP → BIOS, or "control on startup" is off and they haven't engaged yet.
   * Pushed to the backend control loop (which then asserts nothing). Cleared
   * (re-engaging control) only by an explicit user action: changing a fan's
   * speed or mode. Never set on navigation, mount, or unmount.
   */
  released: boolean;
  mappingChannel: number | null;
  sweepingChannel: number | null;
  /** True while a simultaneous "Calibrate all" sweep runs (all mapped fans). */
  sweepingAll: boolean;
  sweepResults: Record<number, SweepResult>;
  /** Live sweep progress per channel (cleared when a sweep starts). */
  sweepProgress: Record<number, SweepProgress>;

  // Burst auto-detect (triggered app-wide on startup; see App.tsx)
  /** A burst run is in flight. */
  burstDetecting: boolean;
  /** Burst has run (or been refused fatally) this session — don't auto-retry. */
  burstDone: boolean;
  /** Latest live burst sample, streamed for the debug modal. Null until a run. */
  burstProgress: BurstProgress | null;

  // Profile & mode state
  profiles: FanProfile[];
  activeProfileId: string;
  fanModes: Record<number, FanMode>;
  /** User-renamed fans, keyed by tach channel. Falls back to the header label. */
  fanNames: Record<number, string>;
  lastCommandedPct: Record<number, number>;

  // Actions — hardware
  refresh: () => Promise<void>;
  /** Rebuild and push the background control plan to the backend control loop. */
  pushControlPlan: () => void;
  stop: () => Promise<void>;
  confirmChannel: (index: number) => Promise<void>;
  mapHeader: (rpmChannel: number) => Promise<void>;
  setSpeed: (rpmChannel: number, pct: number) => Promise<void>;
  sweep: (rpmChannel: number) => Promise<void>;
  /** Calibrate every mapped fan simultaneously (one shared duty ladder). */
  sweepAll: () => Promise<void>;
  cancelSweep: () => Promise<void>;
  runBurstDetect: () => Promise<void>;

  // Actions — profiles
  setActiveProfile: (id: string) => void;
  /** Set the active profile AND drive every mapped fan onto it (curve mode). */
  applyProfileToAll: (id: string) => void;
  renameFan: (rpmChannel: number, name: string) => void;
  setFanMode: (rpmChannel: number, mode: FanMode) => void;
  updateProfilePoints: (profileId: string, points: CurvePoint[]) => void;
  setProfileSource: (profileId: string, source: TempSourceKey) => void;
  addProfile: (name: string, fromProfileId?: string) => string;
  deleteProfile: (profileId: string) => void;
  renameProfile: (profileId: string, name: string) => void;
}

const saved = loadSaved();

function persist(store: FansStore) {
  savePersisted({
    customProfiles: store.profiles.filter((p) => !p.isBuiltin),
    fanModes: store.fanModes,
    activeProfileId: store.activeProfileId,
    fanNames: store.fanNames,
  });
}

/**
 * Translate the store's profile/mode state into the backend control plan. The
 * backend control loop owns this from the moment it's pushed and applies it to
 * whatever fans it has mapped — so the plan depends only on config (modes,
 * profiles, released), never on the live mapping list.
 */
function buildControlPlan(store: FansStore): FanControlPlan {
  const resolve = (mode: FanMode): ControlMode => {
    if (mode.type === "manual") return { type: "manual", pct: mode.pct };
    const profile =
      store.profiles.find((p) => p.id === mode.profileId) ??
      store.profiles.find((p) => p.id === "balanced") ??
      BUILTIN_PROFILES[1];
    return {
      type: "curve",
      tempSource: profile.tempSource,
      points: profile.points.map((p) => ({ tempC: p.tempC, speedPct: p.speedPct })),
    };
  };

  const modes: Record<number, ControlMode> = {};
  for (const [ch, mode] of Object.entries(store.fanModes)) {
    modes[Number(ch)] = resolve(mode);
  }

  return {
    released: store.released,
    // Mapped-but-unassigned fans (e.g. just burst-detected) follow Balanced,
    // matching the historical per-fan default.
    defaultMode: resolve({ type: "curve", profileId: "balanced" }),
    modes,
  };
}

export const useFans = create<FansStore>((set, get) => ({
  // Hardware
  status: null,
  readings: [],
  temps: [],
  loaded: false,
  stopping: false,
  // Start released unless the user opted into engaging control on startup; the
  // startup flow (and any explicit fan action) clears it to engage.
  released: !useSettings.getState().fanControlOnStartup,
  mappingChannel: null,
  sweepingChannel: null,
  sweepingAll: false,
  sweepResults: {},
  sweepProgress: {},

  // Burst auto-detect
  burstDetecting: false,
  burstDone: false,
  burstProgress: null,

  // Profiles
  profiles: [...BUILTIN_PROFILES, ...(saved.customProfiles ?? [])],
  activeProfileId: saved.activeProfileId ?? "balanced",
  fanModes: saved.fanModes ?? {},
  fanNames: saved.fanNames ?? {},
  lastCommandedPct: {},

  refresh: async () => {
    // Display-only now: fetch live status/RPM/temps for whatever page is open.
    // Actual fan *control* runs in the backend control loop (immune to webview
    // timer throttling); this no longer asserts duties or feeds the watchdog.
    //
    // While a sweep or burst runs, the backend holds the chip lock; reading
    // through it would block a worker and pile up polls. The calibration/burst
    // modals update from their own progress events, so skip the polled read.
    if (get().sweepingChannel !== null || get().sweepingAll) return;
    if (get().burstDetecting) return;

    try {
      const snapshot = await fanApi.fanSnapshot();
      set({
        status: snapshot.status,
        readings: snapshot.readings,
        temps: snapshot.temps,
        loaded: true,
      });
    } catch {
      // Keep the last good readings; just synthesize an unavailable status if we
      // never got one, so the UI can leave its loading state.
      if (!get().status) {
        set({
          status: {
            available: false,
            detail: "fan control unavailable",
            chip: null,
            confirmedRpmChannels: [],
            writesEnabled: false,
            manualActive: false,
            mappings: [],
          },
        });
      }
      set({ loaded: true });
    }
  },

  pushControlPlan: () => {
    void fanApi.fanSetControlPlan(buildControlPlan(get())).catch(() => {});
  },

  stop: async () => {
    // Mark released and push it to the backend loop BEFORE the stop, so the loop
    // can't re-assert control between the release and the state settling.
    set({ stopping: true, released: true, lastCommandedPct: {} });
    get().pushControlPlan();
    try {
      await fanApi.fanStop();
      await get().refresh();
    } finally {
      set({ stopping: false });
    }
  },

  confirmChannel: async (index) => {
    await fanApi.fanConfirmChannel(index);
    await get().refresh();
  },

  mapHeader: async (rpmChannel) => {
    set({ mappingChannel: rpmChannel });
    try {
      await fanApi.fanMapHeader(rpmChannel);
      const fanModes = {
        ...get().fanModes,
        [rpmChannel]: { type: "curve" as const, profileId: "balanced" },
      };
      set({ fanModes });
      persist(get());
      get().pushControlPlan();
      await get().refresh();
    } finally {
      set({ mappingChannel: null });
    }
  },

  setSpeed: async (rpmChannel, pct) => {
    // An explicit speed change re-engages control after a STOP → BIOS. Write the
    // duty now for instant feedback; the backend loop holds it from here.
    set((s) => ({
      released: false,
      lastCommandedPct: { ...s.lastCommandedPct, [rpmChannel]: pct },
    }));
    await fanApi.fanSetSpeed(rpmChannel, pct).catch(() => {});
    get().pushControlPlan();
    await get().refresh();
  },

  sweep: async (rpmChannel) => {
    set((s) => ({
      sweepingChannel: rpmChannel,
      sweepProgress: { ...s.sweepProgress, [rpmChannel]: undefined as unknown as SweepProgress },
    }));
    try {
      const result = await fanApi.fanSweep(rpmChannel);
      set((s) => ({ sweepResults: { ...s.sweepResults, [rpmChannel]: result } }));
    } catch (e) {
      // A user Stop returns a "sweep cancelled" error — that's expected, not a
      // failure. Anything else is a real sweep error worth logging.
      if (!String(e).toLowerCase().includes("cancel")) {
        console.error("fan sweep failed", e);
      }
    } finally {
      // Clear the channel BEFORE refreshing so `refresh` doesn't skip itself, and
      // so the UI reflects the fan being handed back to the BIOS curve.
      set({ sweepingChannel: null });
      await get().refresh().catch(() => {});
    }
  },

  sweepAll: async () => {
    if (get().sweepingAll || get().sweepingChannel !== null) return;
    // Fresh progress map so the modal starts empty for every fan.
    set({ sweepingAll: true, sweepProgress: {} });
    try {
      const results = await fanApi.fanSweepAll();
      set((s) => ({
        sweepResults: { ...s.sweepResults, ...Object.fromEntries(results) },
      }));
    } catch (e) {
      // A user Stop returns a "sweep cancelled" error — expected, not a failure.
      if (!String(e).toLowerCase().includes("cancel")) {
        console.error("fan sweep-all failed", e);
      }
    } finally {
      // Clear BEFORE refreshing so `refresh` doesn't skip itself, and so the UI
      // reflects the fans being handed back to the BIOS curve.
      set({ sweepingAll: false });
      await get().refresh().catch(() => {});
    }
  },

  cancelSweep: async () => {
    await fanApi.fanCancelSweep().catch(() => {});
  },

  runBurstDetect: async () => {
    if (get().burstDetecting) return;
    set({ burstDetecting: true, burstProgress: null });
    try {
      await fanApi.fanBurstDetect(useSettings.getState().burstSeconds);
      // Burst ends with every header handed back to the BIOS. Engage control
      // (unless the user opted out of control-on-startup, and hasn't already
      // engaged) and push the plan so the backend loop pulls each freshly
      // detected fan into its saved profile and holds it there.
      const engage = useSettings.getState().fanControlOnStartup;
      set((s) => ({
        burstDone: true,
        released: s.released && !engage,
        lastCommandedPct: {},
      }));
      get().pushControlPlan();
      await get().refresh();
    } catch (e) {
      const msg = String(e);
      // Conflicts not cleared yet — leave burstDone false so the startup
      // trigger retries once the conflict guard is resolved. Any other failure
      // is terminal for this session (don't spin forever on a real error).
      if (!msg.toLowerCase().includes("conflicting")) {
        set({ burstDone: true });
        console.error("fan burst detect failed", e);
      }
    } finally {
      set({ burstDetecting: false });
    }
  },

  setActiveProfile: (id) => {
    set({ activeProfileId: id });
    persist(get());
  },

  applyProfileToAll: (id) => {
    set((s) => {
      const modes: Record<number, FanMode> = { ...s.fanModes };
      for (const m of s.status?.mappings ?? []) {
        modes[m.rpmChannel] = { type: "curve", profileId: id };
      }
      // Picking a global profile re-engages control after a STOP → BIOS.
      return { activeProfileId: id, fanModes: modes, released: false };
    });
    persist(get());
    get().pushControlPlan();
  },

  renameFan: (rpmChannel, name) => {
    set((s) => {
      const fanNames = { ...s.fanNames };
      const trimmed = name.trim();
      if (trimmed) fanNames[rpmChannel] = trimmed;
      else delete fanNames[rpmChannel];
      return { fanNames };
    });
    persist(get());
  },

  setFanMode: (rpmChannel, mode) => {
    const fanModes = { ...get().fanModes, [rpmChannel]: mode };
    // Picking a mode re-engages control after a STOP → BIOS.
    set({ fanModes, released: false });
    persist(get());
    get().pushControlPlan();
  },

  updateProfilePoints: (profileId, points) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, points } : p)),
    }));
    persist(get());
    // Push the edited curve so the backend loop re-evaluates affected fans.
    get().pushControlPlan();
  },

  setProfileSource: (profileId, source) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, tempSource: source } : p)),
    }));
    persist(get());
    get().pushControlPlan();
  },

  addProfile: (name, fromProfileId) => {
    const id = `custom-${Date.now()}`;
    const source =
      get().profiles.find((p) => p.id === (fromProfileId ?? "balanced")) ??
      BUILTIN_PROFILES[1];
    const newProfile: FanProfile = {
      id,
      name,
      isBuiltin: false,
      tempSource: source.tempSource,
      points: source.points.map((p) => ({ ...p })),
    };
    set((s) => ({ profiles: [...s.profiles, newProfile], activeProfileId: id }));
    persist(get());
    return id;
  },

  deleteProfile: (profileId) => {
    if (BUILTIN_PROFILES.some((p) => p.id === profileId)) return;
    set((s) => {
      const profiles = s.profiles.filter((p) => p.id !== profileId);
      const activeProfileId =
        s.activeProfileId === profileId ? "balanced" : s.activeProfileId;
      const fanModes = Object.fromEntries(
        Object.entries(s.fanModes).map(([k, m]) =>
          m.type === "curve" && m.profileId === profileId
            ? [k, { type: "curve" as const, profileId: "balanced" }]
            : [k, m],
        ),
      );
      return { profiles, activeProfileId, fanModes };
    });
    persist(get());
    // Fans on the deleted profile were reassigned to Balanced — re-push.
    get().pushControlPlan();
  },

  renameProfile: (profileId, name) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, name } : p)),
    }));
    persist(get());
  },
}));

/**
 * True only when every controllable fan is under the app (none left on the BIOS
 * curve). The pump is excluded — it's intentionally never app-controlled. Drives
 * the green status dots in the sidebar and fan page: any fan on BIOS → not green.
 */
export function fansFullyControlled(s: FansStore): boolean {
  const st = s.status;
  if (!st?.available || s.released) return false;
  if (st.mappings.length === 0) return false;
  return st.manualActive;
}

// Stream live sweep progress from the backend into the store.
void listen<SweepProgress>("fan-sweep-progress", (e) => {
  const p = e.payload;
  useFans.setState((s) => ({
    sweepProgress: { ...s.sweepProgress, [p.rpmChannel]: p },
  }));
});

// Stream live burst auto-detect progress (drives the debug modal).
void listen<BurstProgress>("fan-burst-progress", (e) => {
  useFans.setState({ burstProgress: e.payload });
});
