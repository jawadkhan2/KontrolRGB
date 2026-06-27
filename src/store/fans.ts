import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import * as fanApi from "../lib/fanApi";
import { useSettings } from "./settings";
import type {
  BurstProgress,
  ChannelReading,
  FanStatus,
  SweepProgress,
  SweepResult,
  TempReading,
} from "../lib/fanApi";

// --- Types ---

export type TempSourceKey = "cpu" | "aux0" | "aux1" | "aux2" | "aux3";

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
  mappingChannel: number | null;
  sweepingChannel: number | null;
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
  lastCommandedPct: Record<number, number>;

  // Actions — hardware
  refresh: () => Promise<void>;
  stop: () => Promise<void>;
  confirmChannel: (index: number) => Promise<void>;
  mapHeader: (rpmChannel: number) => Promise<void>;
  setSpeed: (rpmChannel: number, pct: number) => Promise<void>;
  sweep: (rpmChannel: number) => Promise<void>;
  cancelSweep: () => Promise<void>;
  runBurstDetect: () => Promise<void>;

  // Actions — profiles
  setActiveProfile: (id: string) => void;
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
  });
}

export const useFans = create<FansStore>((set, get) => ({
  // Hardware
  status: null,
  readings: [],
  temps: [],
  loaded: false,
  stopping: false,
  mappingChannel: null,
  sweepingChannel: null,
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
  lastCommandedPct: {},

  refresh: async () => {
    // While a sweep runs, the backend holds the chip lock for minutes; every
    // fanStatus/fanRead would block a worker until it releases. Skip the polled
    // hardware reads — the calibration modal updates from `fan-sweep-progress`
    // events instead — so we don't pile up hundreds of blocked calls.
    if (get().sweepingChannel !== null) return;

    let status: FanStatus;
    let readings: ChannelReading[] = [];
    let temps: TempReading[] = [];
    try {
      const snapshot = await fanApi.fanSnapshot();
      status = snapshot.status;
      readings = snapshot.readings;
      temps = snapshot.temps;
    } catch {
      const current = get();
      status =
        current.status ?? {
          available: false,
          detail: "fan control unavailable",
          chip: null,
          confirmedRpmChannels: [],
          writesEnabled: false,
          manualActive: false,
          mappings: [],
        };
      readings = current.readings;
      temps = current.temps;
    }

    if (status.manualActive) {
      void fanApi.fanHeartbeat().catch(() => {});
    }

    // Auto-apply curve control for each mapped fan
    const { profiles, fanModes, lastCommandedPct } = get();
    const newLastCommanded = { ...lastCommandedPct };

    for (const mapping of status.mappings) {
      const mode = fanModes[mapping.rpmChannel] ?? { type: "curve", profileId: "balanced" };
      if (mode.type !== "curve") continue;
      const profile = profiles.find((p) => p.id === mode.profileId);
      if (!profile) continue;
      const tempReading = temps.find((t) => t.key === profile.tempSource);
      if (!tempReading) continue;

      const rawTarget = interpolateCurve(profile.points, tempReading.tempC);
      const clamped = Math.max(mapping.minPwm, Math.min(mapping.maxPwm, rawTarget));
      const last = lastCommandedPct[mapping.rpmChannel] ?? -99;
      if (Math.abs(clamped - last) > 2) {
        newLastCommanded[mapping.rpmChannel] = clamped;
        void fanApi.fanSetSpeed(mapping.rpmChannel, clamped).catch(() => {});
      }
    }

    set({ status, readings, temps, loaded: true, lastCommandedPct: newLastCommanded });
  },

  stop: async () => {
    set({ stopping: true, lastCommandedPct: {} });
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
      await get().refresh();
    } finally {
      set({ mappingChannel: null });
    }
  },

  setSpeed: async (rpmChannel, pct) => {
    await fanApi.fanSetSpeed(rpmChannel, pct);
    set((s) => ({
      lastCommandedPct: { ...s.lastCommandedPct, [rpmChannel]: pct },
    }));
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

  cancelSweep: async () => {
    await fanApi.fanCancelSweep().catch(() => {});
  },

  runBurstDetect: async () => {
    if (get().burstDetecting) return;
    set({ burstDetecting: true, burstProgress: null });
    try {
      await fanApi.fanBurstDetect(useSettings.getState().burstSeconds);
      set({ burstDone: true });
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

  setFanMode: (rpmChannel, mode) => {
    const fanModes = { ...get().fanModes, [rpmChannel]: mode };
    const lastCommandedPct = { ...get().lastCommandedPct, [rpmChannel]: -99 };
    set({ fanModes, lastCommandedPct });
    persist(get());
  },

  updateProfilePoints: (profileId, points) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, points } : p)),
    }));
    // Force re-apply on next refresh
    set((s) => ({
      lastCommandedPct: Object.fromEntries(
        Object.entries(s.lastCommandedPct).map(([k, v]) => {
          const mode = s.fanModes[Number(k)];
          return mode?.type === "curve" && mode.profileId === profileId ? [k, -99] : [k, v];
        }),
      ),
    }));
    persist(get());
  },

  setProfileSource: (profileId, source) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, tempSource: source } : p)),
    }));
    set((s) => ({
      lastCommandedPct: Object.fromEntries(
        Object.entries(s.lastCommandedPct).map(([k, v]) => {
          const mode = s.fanModes[Number(k)];
          return mode?.type === "curve" && mode.profileId === profileId ? [k, -99] : [k, v];
        }),
      ),
    }));
    persist(get());
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
  },

  renameProfile: (profileId, name) => {
    set((s) => ({
      profiles: s.profiles.map((p) => (p.id === profileId ? { ...p, name } : p)),
    }));
    persist(get());
  },
}));

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
