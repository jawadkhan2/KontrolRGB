//! Device/effect types now live in the shared `kontrolrgb-effects` crate
//! (also compiled to WASM by the kontrolrgb-sim web preview). Re-exported
//! here so existing `crate::device::types::*` paths keep working.

pub use kontrolrgb_effects::types::*;
