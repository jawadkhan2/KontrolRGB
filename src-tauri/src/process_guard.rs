use serde::Serialize;
use sysinfo::{ProcessesToUpdate, System};

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct ConflictProcess {
    pub pid: u32,
    pub display_name: &'static str,
    pub exe_name: String,
}

const KNOWN: &[(&str, &str)] = &[
    ("signalrgb.exe", "SignalRGB"),
    ("signalrgbservice.exe", "SignalRGB Service"),
    ("openrgb.exe", "OpenRGB"),
    ("icue.exe", "Corsair iCUE"),
    ("cue.exe", "Corsair CUE"),
    ("razersynapse.exe", "Razer Synapse"),
    ("lightingservice.exe", "ASUS Lighting Service"),
    ("armourycrate.service.exe", "ASUS Armoury Crate"),
    ("gloriouscore.exe", "Glorious Core"),
    ("rgbfusion.exe", "Gigabyte RGB Fusion"),
    ("rgbfusion2.exe", "Gigabyte RGB Fusion 2.0"),
    ("lghub.exe", "Logitech G Hub"),
    ("gcoreserver.exe", "Logitech G Hub Server"),
    ("nzxt cam.exe", "NZXT CAM"),
    ("dragoncenter.exe", "MSI Dragon Center"),
    ("msicenter.exe", "MSI Center"),
    ("steelseriesengine3.exe", "SteelSeries Engine"),
    ("steelseriesgg.exe", "SteelSeries GG"),
    ("masterplus.exe", "Cooler Master MasterPlus"),
    ("speedfan.exe", "SpeedFan"),
    ("argusmonitor.exe", "Argus Monitor"),
    ("fan control.exe", "Fan Control"),
    ("fancontrol.exe", "Fan Control"),
];

pub fn scan() -> Vec<ConflictProcess> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut found = Vec::new();
    for (pid, proc) in sys.processes() {
        let exe_lower = proc.name().to_string_lossy().to_lowercase();
        if let Some(&(_, display)) = KNOWN.iter().find(|(k, _)| *k == exe_lower) {
            found.push(ConflictProcess {
                pid: pid.as_u32(),
                display_name: display,
                exe_name: proc.name().to_string_lossy().into_owned(),
            });
        }
    }
    found
}

pub fn kill(pids: &[u32]) -> Result<(), String> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut errors = Vec::new();
    for &pid in pids {
        let spid = sysinfo::Pid::from_u32(pid);
        if let Some(proc) = sys.process(spid) {
            if !proc.kill() {
                errors.push(format!("failed to kill PID {pid}"));
            }
        }
        // Process already gone — treat as success
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}
