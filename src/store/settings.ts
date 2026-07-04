import { create } from "zustand";
import type { Color } from "../types/device";

// Global, app-wide settings — deliberately separate from any one feature store
// so future settings (themes, startup behaviour, telemetry, etc.) all live in
// one place and one persisted blob. Add a field here + a row in SettingsPage.

const STORAGE_KEY = "kontrolrgb-settings-v1";
// Legacy: burstOnStartup used to live in the fan store's persisted state.
const LEGACY_FAN_KEY = "kontrolrgb-fan-state-v1";

/** How the Sync page treats devices that can't host-animate the chosen effect
 *  (e.g. the GMMK keyboard): run the closest firmware effect, or leave them out. */
export type SyncFallbackMode = "fallback" | "exclude";

/** The four host-animated effects the Sync page can apply to every device. */
export type SyncEffectId =
  | "static"
  | "breathing"
  | "rainbow_wave"
  | "color_cycle"
  | "meteor"
  | "fire"
  | "twinkle"
  | "gradient"
  | "plasma"
  | "larson"
  | "theater_chase"
  | "ripple";

interface PersistedSettings {
  /**
   * Engage background fan control on startup. When true, the backend control
   * loop takes mapped fans off the BIOS and onto their saved profiles as soon as
   * the chip is ready. When false, fans stay on the BIOS curve until the user
   * changes a fan's mode or speed (which re-engages control).
   */
  fanControlOnStartup: boolean;
  /** Auto-detect fans on startup by briefly bursting every header to 100%. */
  burstOnStartup: boolean;
  /** Show the live burst debug modal while startup auto-detect runs. */
  burstDebug: boolean;
  /** How long (seconds) to hold the startup burst before snapshotting fans. */
  burstSeconds: number;
  /** Sync page handling of devices that can't host-animate the chosen effect. */
  syncFallbackMode: SyncFallbackMode;
  /** When true, at startup the app checks whether it's elevated and — if not —
   *  relaunches itself as administrator (pops UAC). Off by default; some
   *  hardware paths (ring-0 fan driver) need admin, so power users enable it. */
  askAdminOnStartup: boolean;
  /** Sync page: last-chosen effect, color, speed, direction, and per-device
   *  include map. Persisted so the landing page comes back exactly as left,
   *  across page changes, sessions, and app restarts. */
  syncEffectId: SyncEffectId;
  syncColor: Color;
  syncSpeed: number;
  syncReverse: boolean;
  /** Devices the user excluded from the sync. Keyed by device id; missing or
   *  false = included. */
  syncExcluded: Record<string, boolean>;
  /** Effect kinds starred in the Effects Library. Order = order starred. */
  favoriteEffects: string[];
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
  setFanControlOnStartup: (enabled: boolean) => void;
  setAskAdminOnStartup: (enabled: boolean) => void;
  setBurstOnStartup: (enabled: boolean) => void;
  setBurstDebug: (enabled: boolean) => void;
  setBurstSeconds: (secs: number) => void;
  setSyncFallbackMode: (mode: SyncFallbackMode) => void;
  setSyncEffectId: (id: SyncEffectId) => void;
  setSyncColor: (color: Color) => void;
  setSyncSpeed: (speed: number) => void;
  setSyncReverse: (reverse: boolean) => void;
  setSyncExcluded: (excluded: Record<string, boolean>) => void;
  toggleFavoriteEffect: (kind: string) => void;
}

const saved = loadSaved();

function persist(get: () => SettingsStore) {
  const s = get();
  save({
    fanControlOnStartup: s.fanControlOnStartup,
    askAdminOnStartup: s.askAdminOnStartup,
    burstOnStartup: s.burstOnStartup,
    burstDebug: s.burstDebug,
    burstSeconds: s.burstSeconds,
    syncFallbackMode: s.syncFallbackMode,
    syncEffectId: s.syncEffectId,
    syncColor: s.syncColor,
    syncSpeed: s.syncSpeed,
    syncReverse: s.syncReverse,
    syncExcluded: s.syncExcluded,
    favoriteEffects: s.favoriteEffects,
  });
}

export const useSettings = create<SettingsStore>((set, get) => ({
  fanControlOnStartup: saved.fanControlOnStartup ?? true,
  askAdminOnStartup: saved.askAdminOnStartup ?? false,
  burstOnStartup: saved.burstOnStartup ?? true,
  burstDebug: saved.burstDebug ?? true,
  burstSeconds: saved.burstSeconds ?? DEFAULT_BURST_SECONDS,
  syncFallbackMode: saved.syncFallbackMode ?? "fallback",
  syncEffectId: saved.syncEffectId ?? "rainbow_wave",
  syncColor: saved.syncColor ?? { r: 91, g: 140, b: 255 },
  syncSpeed: saved.syncSpeed ?? 55,
  syncReverse: saved.syncReverse ?? false,
  syncExcluded: saved.syncExcluded ?? {},
  favoriteEffects: saved.favoriteEffects ?? [],

  setFanControlOnStartup: (enabled) => {
    set({ fanControlOnStartup: enabled });
    persist(get);
  },

  setAskAdminOnStartup: (enabled) => {
    set({ askAdminOnStartup: enabled });
    persist(get);
  },

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
  setSyncFallbackMode: (mode) => {
    set({ syncFallbackMode: mode });
    persist(get);
  },
  setSyncEffectId: (id) => {
    set({ syncEffectId: id });
    persist(get);
  },
  setSyncColor: (color) => {
    set({ syncColor: color });
    persist(get);
  },
  setSyncSpeed: (speed) => {
    set({ syncSpeed: speed });
    persist(get);
  },
  setSyncReverse: (reverse) => {
    set({ syncReverse: reverse });
    persist(get);
  },
  setSyncExcluded: (excluded) => {
    set({ syncExcluded: excluded });
    persist(get);
  },
  toggleFavoriteEffect: (kind) => {
    const favs = get().favoriteEffects;
    set({
      favoriteEffects: favs.includes(kind)
        ? favs.filter((k) => k !== kind)
        : [...favs, kind],
    });
    persist(get);
  },
}));
