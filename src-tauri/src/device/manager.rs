use super::types::{DeviceId, DeviceInfo};
use super::{gmmk, mock, msi, RgbDevice};

pub struct DeviceManager {
    devices: Vec<Box<dyn RgbDevice>>,
}

impl DeviceManager {
    pub fn new() -> Self {
        let mut m = DeviceManager {
            devices: Vec::new(),
        };
        m.rescan();
        m
    }

    /// Re-detect devices. Each real backend probes here; the mock stands in
    /// when the hardware is absent (or the backend isn't written yet).
    pub fn rescan(&mut self) {
        // Drop old devices first so their HID handles/worker threads close
        // before re-probing.
        self.devices.clear();

        match gmmk::probe() {
            Ok(Some(dev)) => self.devices.push(Box::new(dev)),
            Ok(None) => {
                eprintln!("gmmk: keyboard not found, using mock");
                self.devices.push(Box::new(mock::mock_gmmk()));
            }
            Err(e) => {
                eprintln!("gmmk: probe failed ({e}), using mock");
                self.devices.push(Box::new(mock::mock_gmmk()));
            }
        }

        match msi::probe() {
            Ok(Some(dev)) => self.devices.push(Box::new(dev)),
            Ok(None) => {
                eprintln!("msi: board not found, using mock");
                self.devices.push(Box::new(mock::mock_msi_z890()));
            }
            Err(e) => {
                eprintln!("msi: probe failed ({e}), using mock");
                self.devices.push(Box::new(mock::mock_msi_z890()));
            }
        }

        self.scan_gpu();
    }

    /// Probe the Gigabyte GPU (M4). NvAPI is Windows-only, so non-Windows
    /// builds and machines without the card fall back to the mock.
    fn scan_gpu(&mut self) {
        #[cfg(windows)]
        match super::gigabyte_gpu::probe() {
            Ok(Some(dev)) => {
                self.devices.push(Box::new(dev));
                return;
            }
            Ok(None) => eprintln!("gigabyte-gpu: card not found, using mock"),
            Err(e) => eprintln!("gigabyte-gpu: probe failed ({e}), using mock"),
        }

        self.devices.push(Box::new(mock::mock_rtx5080()));
    }

    pub fn infos(&self) -> Vec<DeviceInfo> {
        self.devices.iter().map(|d| d.info()).collect()
    }

    pub fn get_mut(&mut self, id: &DeviceId) -> Option<&mut Box<dyn RgbDevice>> {
        self.devices.iter_mut().find(|d| d.id() == id)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn RgbDevice>> {
        self.devices.iter_mut()
    }
}
