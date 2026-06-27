import { create } from "zustand";

// Global, app-wide settings — deliberately separate from any one feature store
// so future settings (themes, startup behaviour, telemetry, etc.) all live in
// one place and one persisted blob. Add a field here + a row in SettingsPage.

const STORAGE_KEY = "kontrolrgb-settings-v1";
// Legacy: burstOnStartup used to live in the fan store's persisted state.
const LEGACY_FAN_KEY = "kontrolrgb-fan-state-v1";

interface PersistedSettings {
  /** Auto-detect fans on startup by briefly bursting every header to 100%. */
  burstOnStartup: boolean;
  /** Show the live burst debug modal while startup auto-detect runs. */
  burstDebug: boolean;
  /** How long (seconds) to hold the startup burst before snapshotting fans. */
  burstSeconds: number;
}

/** Default burst hold; long enough for slow movers (SYS_FAN 5/6) to spin up. */
export const DEFAULT_BURST_SECONDS = 8;

function loadSaved(): Partial<PersistedSettings> {
  try {
    const s = localStorage.getItem(STORAGE_KEY);
    if (s) return JSON.parse(s) as Partial<PersistedSettings>;
  } catch {}
  // One-time migration from the old fan-store location.
  try {
    const legacy = localStorage.getItem(LEGACY_FAN_KEY);
    if (legacy) {
      const v = JSON.parse(legacy) as { burstOnStartup?: boolean };
      if (typeof v.burstOnStartup === "boolean") {
        return { burstOnStartup: v.burstOnStartup };
      }
    }
  } catch {}
  return {};
}

function save(state: PersistedSettings) {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(state));
  } catch {}
}

interface SettingsStore extends PersistedSettings {
  setBurstOnStartup: (enabled: boolean) => void;
  setBurstDebug: (enabled: boolean) => void;
  setBurstSeconds: (secs: number) => void;
}

const saved = loadSaved();

function persist(get: () => SettingsStore) {
  const s = get();
  save({
    burstOnStartup: s.burstOnStartup,
    burstDebug: s.burstDebug,
    burstSeconds: s.burstSeconds,
  });
}

export const useSettings = create<SettingsStore>((set, get) => ({
  burstOnStartup: saved.burstOnStartup ?? true,
  burstDebug: saved.burstDebug ?? true,
  burstSeconds: saved.burstSeconds ?? DEFAULT_BURST_SECONDS,

  setBurstOnStartup: (enabled) => {
    set({ burstOnStartup: enabled });
    persist(get);
  },
  setBurstDebug: (enabled) => {
    set({ burstDebug: enabled });
    persist(get);
  },
  setBurstSeconds: (secs) => {
    // Keep in the same band the backend clamps to.
    const clamped = Math.max(2, Math.min(30, Math.round(secs)));
    set({ burstSeconds: clamped });
    persist(get);
  },
}));
