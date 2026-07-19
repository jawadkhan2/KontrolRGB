// Single source of truth for every effect the app knows about: label, preview,
// category/tags (library search), which controls it takes, and how it degrades
// to a firmware mode on write-once devices. Adding an effect = one entry here,
// one pv-* class in index.css, and the backend implementation. The EffectPanel,
// SyncPage, and EffectsLibraryPage all derive from this table.

import type {
  Color,
  EffectConfig,
  EffectKind,
  OnboardMode,
} from "../types/device";

export type EffectCategory =
  | "Motion"
  | "Ambient"
  | "Color"
  | "Reactive"
  | "Special";

export const EFFECT_CATEGORIES: EffectCategory[] = [
  "Motion",
  "Ambient",
  "Color",
  "Reactive",
];

export interface EffectMeta {
  kind: EffectKind;
  label: string;
  /** One-liner for the library card. */
  description: string;
  category: EffectCategory;
  /** Extra search terms beyond the label. */
  tags: string[];
  /** Which controls the host effect takes — mirrors the EffectConfig shape. */
  supports: { color: boolean; speed: boolean; direction: boolean };
  /** Animated preview swatch class (pv-* in index.css). */
  pv: string;
  /** Listed in the Effects Library / Sync grids. Custom + Onboard are
   *  device-page modes, not browsable effects. */
  browsable: boolean;
  /** Closest firmware mode for devices that can't host-animate (the GMMK).
   *  `color` pins the firmware color for paletted effects with no color
   *  control, so the keyboard stays in the effect's palette instead of the
   *  user's (unrelated) sync color. */
  onboard: { mode: OnboardMode; rainbow: boolean; color?: Color } | null;
}

export const EFFECTS: EffectMeta[] = [
  {
    kind: "static",
    label: "Static",
    description: "One solid color, held steady.",
    category: "Color",
    tags: ["solid", "fill", "plain", "single"],
    supports: { color: true, speed: false, direction: false },
    pv: "pv-static",
    browsable: true,
    onboard: { mode: "fixed", rainbow: false },
  },
  {
    kind: "breathing",
    label: "Breathing",
    description: "Slow fade in and out of your color.",
    category: "Ambient",
    tags: ["fade", "pulse", "calm"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-breathe",
    browsable: true,
    onboard: { mode: "breathing", rainbow: false },
  },
  {
    kind: "rainbow_wave",
    label: "Rainbow Wave",
    description: "Full spectrum scrolling across the device.",
    category: "Motion",
    tags: ["spectrum", "scroll", "flow", "classic"],
    supports: { color: false, speed: true, direction: true },
    pv: "pv-wave",
    browsable: true,
    onboard: { mode: "wave", rainbow: true },
  },
  {
    kind: "color_cycle",
    label: "Color Cycle",
    description: "Every LED shifts through the spectrum together.",
    category: "Color",
    tags: ["spectrum", "hue", "rotate", "cycle"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-cycle",
    browsable: true,
    onboard: { mode: "spectrum", rainbow: true },
  },
  {
    kind: "meteor",
    label: "Meteor",
    description: "A bright head streaks across with a fading tail.",
    category: "Motion",
    tags: ["comet", "shooting", "trail", "streak"],
    supports: { color: true, speed: true, direction: true },
    pv: "pv-meteor",
    browsable: true,
    onboard: { mode: "wave", rainbow: false },
  },
  {
    kind: "fire",
    label: "Fire",
    description: "Flickering embers in reds and ambers.",
    category: "Ambient",
    tags: ["flame", "ember", "heat", "flicker"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-fire",
    browsable: true,
    onboard: { mode: "breathing", rainbow: false },
  },
  {
    kind: "twinkle",
    label: "Twinkle",
    description: "Random LEDs sparkle over your base color.",
    category: "Ambient",
    tags: ["sparkle", "stars", "glitter", "random"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-twinkle",
    browsable: true,
    onboard: { mode: "reactive", rainbow: false },
  },
  {
    kind: "gradient",
    label: "Gradient",
    description: "A smooth two-tone blend drifting across the zone.",
    category: "Color",
    tags: ["blend", "duotone", "smooth", "shift"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-gradient",
    browsable: true,
    onboard: { mode: "wave", rainbow: false },
  },
  {
    kind: "plasma",
    label: "Plasma",
    description: "Organic swirls of shifting color.",
    category: "Ambient",
    tags: ["swirl", "lava", "organic", "noise"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-plasma",
    browsable: true,
    onboard: { mode: "spectrum", rainbow: true },
  },
  {
    kind: "larson",
    label: "Larson",
    description: "A scanner eye sweeping back and forth.",
    category: "Motion",
    tags: ["scanner", "kitt", "cylon", "sweep"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-larson",
    browsable: true,
    onboard: { mode: "wave", rainbow: false },
  },
  {
    kind: "theater_chase",
    label: "Theater Chase",
    description: "Marquee dots marching in step.",
    category: "Motion",
    tags: ["marquee", "dots", "march", "chase"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-chase",
    browsable: true,
    onboard: { mode: "wave", rainbow: false },
  },
  {
    kind: "ripple",
    label: "Ripple",
    description: "Rings of color expanding from the center.",
    category: "Motion",
    tags: ["rings", "waves", "expand", "pulse"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-ripple",
    browsable: true,
    onboard: { mode: "swirl", rainbow: true },
  },
  {
    kind: "aurora",
    label: "Aurora",
    description: "Northern-lights curtains drifting in teal and violet.",
    category: "Ambient",
    tags: ["northern", "lights", "borealis", "curtain", "arctic"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-aurora",
    browsable: true,
    onboard: { mode: "wave", rainbow: false, color: { r: 40, g: 235, b: 160 } },
  },
  {
    kind: "vortex",
    label: "Vortex",
    description: "Bright arcs orbiting each fan ring.",
    category: "Motion",
    tags: ["spin", "orbit", "rotate", "spiral", "radar"],
    supports: { color: true, speed: true, direction: true },
    pv: "pv-vortex",
    browsable: true,
    onboard: { mode: "swirl", rainbow: false },
  },
  {
    kind: "heartbeat",
    label: "Heartbeat",
    description: "A double-thump pulse, every device beating in sync.",
    category: "Ambient",
    tags: ["pulse", "beat", "cardiac", "thump", "rhythm"],
    supports: { color: true, speed: true, direction: false },
    pv: "pv-heartbeat",
    browsable: true,
    onboard: { mode: "breathing", rainbow: false },
  },
  {
    kind: "thunderstorm",
    label: "Thunderstorm",
    description: "Dark storm blue split by flickering lightning strikes.",
    category: "Ambient",
    tags: ["lightning", "storm", "rain", "flash", "bolt"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-storm",
    browsable: true,
    onboard: { mode: "reactive", rainbow: false, color: { r: 150, g: 180, b: 255 } },
  },
  {
    kind: "sunset",
    label: "Sunset",
    description: "A dusk gradient sinking from violet sky to golden horizon.",
    category: "Color",
    tags: ["dusk", "horizon", "warm", "dawn", "gradient", "sky"],
    supports: { color: false, speed: true, direction: false },
    pv: "pv-sunset",
    browsable: true,
    onboard: { mode: "wave", rainbow: false, color: { r: 255, g: 120, b: 40 } },
  },
  {
    kind: "custom",
    label: "Custom",
    description: "Paint individual keys or LEDs by hand.",
    category: "Special",
    tags: ["paint", "manual", "per-key"],
    supports: { color: false, speed: false, direction: false },
    pv: "pv-custom",
    browsable: false,
    onboard: null,
  },
  {
    kind: "onboard",
    label: "Onboard",
    description: "The device firmware animates itself.",
    category: "Special",
    tags: ["firmware", "hardware"],
    supports: { color: false, speed: false, direction: false },
    pv: "pv-onboard",
    browsable: false,
    onboard: null,
  },
];

export const EFFECT_BY_KIND = Object.fromEntries(
  EFFECTS.map((e) => [e.kind, e]),
) as Record<EffectKind, EffectMeta>;

export const isEffectKind = (s: string): s is EffectKind => s in EFFECT_BY_KIND;

export const effectLabel = (kind: EffectKind) => EFFECT_BY_KIND[kind].label;

export const DEFAULT_EFFECT_COLOR: Color = { r: 91, g: 140, b: 255 };

/** Default config for a kind, carrying color/speed over from the previous
 *  effect so switching kinds feels continuous. */
export function defaultEffect(kind: EffectKind, prev?: EffectConfig): EffectConfig {
  const color = prev && "color" in prev ? prev.color : DEFAULT_EFFECT_COLOR;
  const speed = prev && "speed" in prev ? prev.speed : 1.0;
  if (kind === "custom") return { kind };
  if (kind === "onboard") {
    return { kind, mode: "wave", color, rainbow: true, speed: 2, reverse: false };
  }
  return hostConfig(kind, { color, speed, reverse: false });
}

/** Build a host-animated EffectConfig from generic control values. The supports
 *  flags mirror the EffectConfig union exactly, so assembling by flag is safe. */
export function hostConfig(
  kind: Exclude<EffectKind, "custom" | "onboard">,
  opts: { color: Color; speed: number; reverse: boolean },
): EffectConfig {
  const m = EFFECT_BY_KIND[kind];
  const cfg: Record<string, unknown> = { kind };
  if (m.supports.color) cfg.color = opts.color;
  if (m.supports.speed) cfg.speed = opts.speed;
  if (m.supports.direction) cfg.reverse = opts.reverse;
  return cfg as EffectConfig;
}

/** native = host can animate it · fallback = closest firmware mode · none. */
export type Capability = "native" | "fallback" | "none";

export function capabilityFor(supported: string[], meta: EffectMeta): Capability {
  if (supported.includes(meta.kind)) return "native";
  if (meta.onboard && supported.includes("onboard")) return "fallback";
  return "none";
}

/** Host effects take a 0.1..5× float; firmware modes take a 0..4 integer. */
export const speedPctToFloat = (pct: number) =>
  Math.min(5, Math.max(0.1, (pct / 100) * 5));
export const speedPctToOnboard = (pct: number) =>
  Math.round((pct / 100) * 4);

/** Resolve an effect for one device: native host config, firmware fallback,
 *  or null when the device can't run it at all. Speed is a 0..100 percent. */
export function configForDevice(
  supported: string[],
  meta: EffectMeta,
  opts: { color: Color; speedPct: number; reverse: boolean },
): EffectConfig | null {
  const cap = capabilityFor(supported, meta);
  if (cap === "native" && meta.kind !== "custom" && meta.kind !== "onboard") {
    return hostConfig(meta.kind, {
      color: opts.color,
      speed: speedPctToFloat(opts.speedPct),
      reverse: opts.reverse,
    });
  }
  if (cap === "fallback" && meta.onboard) {
    return {
      kind: "onboard",
      mode: meta.onboard.mode,
      color: (!meta.supports.color && meta.onboard.color) || opts.color,
      rainbow: meta.onboard.rainbow,
      speed: speedPctToOnboard(opts.speedPct),
      reverse: meta.supports.direction ? opts.reverse : false,
    };
  }
  return null;
}

/** Library search: match label, description, and tags, case-insensitive. */
export function matchesQuery(meta: EffectMeta, query: string): boolean {
  const q = query.trim().toLowerCase();
  if (!q) return true;
  return (
    meta.label.toLowerCase().includes(q) ||
    meta.description.toLowerCase().includes(q) ||
    meta.tags.some((t) => t.toLowerCase().includes(q))
  );
}
