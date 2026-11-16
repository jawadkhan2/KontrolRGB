//! Conflict guard: keeps other RGB / fan software off the hardware.
//!
//! Two processes driving the same HID controller or the same Super-I/O chip
//! silently fight — Windows opens HID devices shared, so neither side ever sees
//! an error. The classic symptom: MSI's `LEDKeeper2` / `Mystic_Light_Service`
//! re-applying the board's rainbow on every wake while KontrolRGB thinks it is
//! still in control.
//!
//! The original guard only ran once at startup, needed the user to click a
//! modal (invisible on a boot launch, where the window stays hidden), and only
//! knew the vendors' *UI* exe names — not the background services that actually
//! touch the hardware. Now:
//! - the list covers the vendor services (MSI Center's included);
//! - with auto-enforce on (default), a background thread re-checks every
//!   `ENFORCE_EVERY` and shuts anything it finds down: services are stopped
//!   through the service manager first (killing a service's process just makes
//!   the SCM restart it) and optionally have their start type set to Disabled,
//!   then remaining processes are killed;
//! - the first pass runs immediately at launch, so hardware writes are enabled
//!   without waiting on the webview.

use std::collections::BTreeSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};
use tauri::{AppHandle, Emitter};

use crate::state::AppState;

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ConflictProcess {
    pub pid: u32,
    pub display_name: &'static str,
    pub exe_name: String,
}

/// User-facing guard settings, persisted in config.json under `guard`.
#[derive(Serialize, Deserialize, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase", default)]
pub struct GuardConfig {
    /// Automatically shut down conflicting software whenever it appears.
    pub auto_enforce: bool,
    /// When stopping a conflicting Windows service, also set it to Disabled so
    /// it doesn't come back at the next boot.
    pub disable_services: bool,
}

impl Default for GuardConfig {
    fn default() -> Self {
        GuardConfig {
            auto_enforce: true,
            disable_services: true,
        }
    }
}

struct Known {
    /// Lower-case process image name.
    exe: &'static str,
    display: &'static str,
    /// Windows service that owns this process, if it is one. Stopped via the
    /// service manager before any kill, or the SCM would just restart it.
    service: Option<&'static str>,
}

const fn k(exe: &'static str, display: &'static str) -> Known {
    Known {
        exe,
        display,
        service: None,
    }
}

const fn svc(exe: &'static str, display: &'static str, service: &'static str) -> Known {
    Known {
        exe,
        display,
        service: Some(service),
    }
}

const KNOWN: &[Known] = &[
    // --- MSI. The UI exe was the only entry before; the services below are what
    // actually re-apply Mystic Light (rainbow) on wake and touch the fan chip.
    k("msicenter.exe", "MSI Center"),
    k("dragoncenter.exe", "MSI Dragon Center"),
    svc(
        "mystic_light_service.exe",
        "MSI Mystic Light Service",
        "Mystic_Light_Service",
    ),
    k("ledkeeper2.exe", "MSI LED Keeper"),
    k("ledkeeper.exe", "MSI LED Keeper"),
    svc(
        "msi_central_service.exe",
        "MSI Center Service",
        "MSI_Center_Service",
    ),
    k("msi.centralserver.exe", "MSI Center Server"),
    k("msi.terminalserver.exe", "MSI Center Terminal Server"),
    svc("msi_case_service.exe", "MSI Case Service", "MSI_Case_Service"),
    // --- Other vendors / tools.
    k("signalrgb.exe", "SignalRGB"),
    k("signalrgbservice.exe", "SignalRGB Service"),
    k("openrgb.exe", "OpenRGB"),
    k("icue.exe", "Corsair iCUE"),
    k("cue.exe", "Corsair CUE"),
    k("razersynapse.exe", "Razer Synapse"),
    svc(
        "lightingservice.exe",
        "ASUS Lighting Service",
        "LightingService",
    ),
    k("armourycrate.service.exe", "ASUS Armoury Crate"),
    k("gloriouscore.exe", "Glorious Core"),
    k("rgbfusion.exe", "Gigabyte RGB Fusion"),
    k("rgbfusion2.exe", "Gigabyte RGB Fusion 2.0"),
    k("lghub.exe", "Logitech G Hub"),
    k("gcoreserver.exe", "Logitech G Hub Server"),
    k("nzxt cam.exe", "NZXT CAM"),
    k("steelseriesengine3.exe", "SteelSeries Engine"),
    k("steelseriesgg.exe", "SteelSeries GG"),
    k("masterplus.exe", "Cooler Master MasterPlus"),
    k("speedfan.exe", "SpeedFan"),
    k("argusmonitor.exe", "Argus Monitor"),
    k("fan control.exe", "Fan Control"),
    k("fancontrol.exe", "Fan Control"),
];

/// Background re-check cadence while auto-enforce is on.
const ENFORCE_EVERY: Duration = Duration::from_secs(15);

fn refresh(sys: &mut System) {
    // Names only — no CPU/memory/disk sampling, keeps each pass cheap.
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::new());
}

fn find(sys: &System) -> Vec<(ConflictProcess, Option<&'static str>)> {
    let mut found = Vec::new();
    for (pid, proc) in sys.processes() {
        let exe_lower = proc.name().to_string_lossy().to_lowercase();
        if let Some(known) = KNOWN.iter().find(|k| k.exe == exe_lower) {
            found.push((
                ConflictProcess {
                    pid: pid.as_u32(),
                    display_name: known.display,
                    exe_name: proc.name().to_string_lossy().into_owned(),
                },
                known.service,
            ));
        }
    }
    found
}

pub fn scan() -> Vec<ConflictProcess> {
    let mut sys = System::new();
    refresh(&mut sys);
    find(&sys).into_iter().map(|(p, _)| p).collect()
}

/// Run `sc.exe` without flashing a console window. Returns whether it exited 0.
#[cfg(windows)]
fn sc(args: &[&str]) -> bool {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("sc.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

#[cfg(not(windows))]
fn sc(_args: &[&str]) -> bool {
    false
}

/// Shut down everything in `found`: stop (and optionally disable) owning
/// services, then kill whatever is still alive. Returns per-process errors.
fn enforce_found(
    sys: &mut System,
    found: &[(ConflictProcess, Option<&'static str>)],
    disable_services: bool,
    verbose: bool,
) -> Vec<String> {
    let services: BTreeSet<&'static str> = found.iter().filter_map(|(_, s)| *s).collect();
    for name in &services {
        if disable_services {
            let ok = sc(&["config", name, "start=", "disabled"]);
            if verbose {
                crate::klog!("guard: disable service {name}: {}", if ok { "ok" } else { "FAILED" });
            }
        }
        let ok = sc(&["stop", name]);
        if verbose {
            crate::klog!("guard: stop service {name}: {}", if ok { "ok" } else { "failed/not running" });
        }
    }
    if !services.is_empty() {
        // Give the SCM a moment to tear the service processes down.
        std::thread::sleep(Duration::from_millis(800));
        refresh(sys);
    }

    let mut errors = Vec::new();
    for (p, _) in found {
        let Some(proc) = sys.process(sysinfo::Pid::from_u32(p.pid)) else {
            continue; // already gone (service stop took it down)
        };
        if proc.kill() {
            if verbose {
                crate::klog!("guard: killed {} (pid {})", p.exe_name, p.pid);
            }
        } else {
            // It may simply have exited since the snapshot — only a process
            // that is still alive counts as a failure.
            let pid = sysinfo::Pid::from_u32(p.pid);
            sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                true,
                ProcessRefreshKind::new(),
            );
            if sys.process(pid).is_some() {
                errors.push(format!("failed to kill {} (PID {})", p.exe_name, p.pid));
            }
        }
    }
    errors
}

/// Serialises enforcement passes. The startup UI check and the background
/// thread both run one at launch; overlapping, they raced on the same PIDs and
/// logged bogus "failed to kill" for processes the other pass had just ended.
static ENFORCE_LOCK: parking_lot::Mutex<()> = parking_lot::Mutex::new(());

/// Explicit kill from the conflict modal (auto-enforce off). Stops services too.
pub fn kill(pids: &[u32], disable_services: bool) -> Result<(), String> {
    let _serial = ENFORCE_LOCK.lock();
    let mut sys = System::new();
    refresh(&mut sys);
    let found: Vec<_> = find(&sys)
        .into_iter()
        .filter(|(p, _)| pids.contains(&p.pid))
        .collect();
    let errors = enforce_found(&mut sys, &found, disable_services, true);
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// One enforcement pass. Returns what was found (empty = clean). Logs only
/// when the set of offenders differs from `last_seen`, so a process we can't
/// kill (not elevated) doesn't spam the log every pass.
pub fn enforce_once(
    sys: &mut System,
    cfg: GuardConfig,
    last_seen: &mut Vec<String>,
) -> (Vec<ConflictProcess>, Vec<String>) {
    let _serial = ENFORCE_LOCK.lock();
    refresh(sys);
    let found = find(sys);
    let mut names: Vec<String> = found.iter().map(|(p, _)| p.exe_name.clone()).collect();
    names.sort();
    let verbose = names != *last_seen;
    *last_seen = names;
    if found.is_empty() {
        return (Vec::new(), Vec::new());
    }
    if verbose {
        crate::klog!("guard: conflicting software running: {}", last_seen.join(", "));
    }
    let errors = enforce_found(sys, &found, cfg.disable_services, verbose);
    if verbose {
        for e in &errors {
            crate::klog!("guard: {e}");
        }
    }
    (found.into_iter().map(|(p, _)| p).collect(), errors)
}

/// Spawn the background enforcer. First pass runs immediately so a boot launch
/// (hidden window, nobody to click the modal) still enables hardware writes.
pub fn spawn_enforcer(app: AppHandle, state: Arc<AppState>) {
    let _ = std::thread::Builder::new()
        .name("conflict-guard".into())
        .spawn(move || {
            let mut sys = System::new();
            let mut last_seen: Vec<String> = Vec::new();
            loop {
                let cfg = *state.guard.lock();
                if cfg.auto_enforce {
                    let (found, errors) = enforce_once(&mut sys, cfg, &mut last_seen);
                    // Auto-enforce means the user pre-approved this; writes are
                    // enabled even if a kill failed (not elevated) — the log and
                    // the event say what is still running.
                    state.conflicts_cleared.store(true, Ordering::Relaxed);
                    if !found.is_empty() {
                        let _ = app.emit("conflicts-enforced", (&found, &errors));
                        // Something was shut down: take back whatever it did to
                        // the hardware. (If nothing could be stopped — not
                        // elevated — re-arming every pass would only flicker.)
                        if errors.len() < found.len() {
                            state.request_resync();
                        }
                    }
                }
                std::thread::sleep(ENFORCE_EVERY);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_exe_names_are_lowercase_and_unique() {
        // `find` lower-cases the live process name before comparing, so an
        // entry with capitals would silently never match.
        let mut seen = BTreeSet::new();
        for k in KNOWN {
            assert_eq!(k.exe, k.exe.to_lowercase(), "{} must be lower-case", k.exe);
            assert!(seen.insert(k.exe), "duplicate entry {}", k.exe);
        }
    }

    #[test]
    fn msi_lighting_services_are_covered() {
        for exe in ["ledkeeper2.exe", "mystic_light_service.exe", "msi_central_service.exe"] {
            assert!(KNOWN.iter().any(|k| k.exe == exe), "{exe} missing");
        }
    }

    #[test]
    fn guard_defaults_to_enforcing() {
        let cfg: GuardConfig = serde_json::from_str("{}").unwrap();
        assert!(cfg.auto_enforce && cfg.disable_services);
    }
}
