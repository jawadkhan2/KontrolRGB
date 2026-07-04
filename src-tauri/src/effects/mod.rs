//! Effect math lives in the shared `kontrolrgb-effects` crate (also compiled
//! to WASM by the kontrolrgb-sim web preview) so the browser preview and the
//! hardware output can never drift. Only the engine loop — which needs tauri,
//! tokio, and the device manager — stays app-side.

pub mod engine;

pub use kontrolrgb_effects::effects::*;
