//! Machine-readable description of every effect kind and its parameters.
//!
//! The kontrolrgb-sim web app generates its whole control panel from this, so
//! adding a new effect variant needs a catalog entry here and NOTHING in the
//! web frontend. Param `id`s must match the serde field names of the
//! corresponding `EffectConfig` variant exactly — the sim builds the effect
//! JSON `{ "kind": <kind>, <id>: <value>, ... }` straight from them.

use serde_json::{json, Value};

/// Param descriptor conventions:
/// - `{"id", "label", "type": "color",  "default": {r,g,b}}`
/// - `{"id", "label", "type": "slider", "min", "max", "step", "default"}`
/// - `{"id", "label", "type": "toggle", "default": bool}`
/// - `{"id", "label", "type": "select", "options": [..], "default": str}`
pub fn effect_catalog() -> Value {
    let color = |id: &str| {
        json!({"id": id, "label": "Color", "type": "color",
               "default": {"r": 255, "g": 40, "b": 120}})
    };
    let speed = || {
        json!({"id": "speed", "label": "Speed", "type": "slider",
               "min": 0.1, "max": 5.0, "step": 0.1, "default": 1.0})
    };
    let reverse = || json!({"id": "reverse", "label": "Reverse", "type": "toggle", "default": false});

    json!([
        {"kind": "static", "name": "Static", "params": [color("color")]},
        {"kind": "breathing", "name": "Breathing", "params": [color("color"), speed()]},
        {"kind": "rainbow_wave", "name": "Rainbow Wave", "params": [speed(), reverse()]},
        {"kind": "color_cycle", "name": "Color Cycle", "params": [speed()]},
        {"kind": "meteor", "name": "Meteor", "params": [color("color"), speed(), reverse()]},
        {"kind": "fire", "name": "Fire", "params": [speed()]},
        {"kind": "twinkle", "name": "Twinkle", "params": [color("color"), speed()]},
        {"kind": "gradient", "name": "Gradient", "params": [color("color"), speed()]},
        {"kind": "plasma", "name": "Plasma", "params": [speed()]},
        {"kind": "larson", "name": "Larson Scanner", "params": [color("color"), speed()]},
        {"kind": "theater_chase", "name": "Theater Chase", "params": [color("color"), speed()]},
        {"kind": "ripple", "name": "Ripple", "params": [speed()]},
        // Host preview of the GMMK firmware modes. Serde flattens OnboardEffect
        // fields next to the tag, so param ids are the OnboardEffect fields.
        {"kind": "onboard", "name": "Onboard (GMMK firmware)", "params": [
            {"id": "mode", "label": "Mode", "type": "select", "default": "wave",
             "options": ["fixed", "breathing", "wave", "spectrum", "reactive", "swirl"]},
            color("color"),
            {"id": "rainbow", "label": "Rainbow", "type": "toggle", "default": true},
            {"id": "speed", "label": "Speed", "type": "slider",
             "min": 0, "max": 4, "step": 1, "default": 2},
            reverse(),
        ]},
    ])
}
