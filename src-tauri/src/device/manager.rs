//! Owns the RGB devices, one per hardware slot (keyboard, motherboard, GPU).
//!
//! Each slot holds either the real backend or a mock stand-in. The important
//! rule: once a slot has real hardware, nothing short of an explicit user
//! Rescan replaces it — real backends heal their own dead handles in place.
//! (Wake recovery used to tear every device down and re-probe; a single
//! transient probe failure then swapped the board for a mock *permanently*,
//! which is exactly "fans stuck on rainbow, GPU fine".) Slots that are mocks
//! can be upgraded to real hardware at any time via `probe_real` + `upgrade`.
//!
//! Probing is deliberately done by associated functions that don't need the
//! manager — callers run the slow HID/NvAPI probes *outside* the manager lock
//! so the effects engine never stalls behind a scan.

use super::types::{DeviceId, DeviceInfo};
use super::{gmmk, mock, msi, RgbDevice};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Slot {
    Keyboard,
    Motherboard,
    Gpu,
}

impl Slot {
    pub const ALL: [Slot; 3] = [Slot::Keyboard, Slot::Motherboard, Slot::Gpu];

    /// Id the real backend for this slot reports. Used to decide whether a
    /// missing slot is hardware the user actually owns (has saved config for),
    /// and therefore worth re-probing in the background.
    pub fn real_id(self) -> String {
        match self {
            Slot::Keyboard => gmmk::device_id(),
            Slot::Motherboard => msi::device_id(),
            #[cfg(windows)]
            Slot::Gpu => super::gigabyte_gpu::device_id(),
            #[cfg(not(windows))]
            Slot::Gpu => "gigabyte-gpu".to_string(),
        }
    }

    fn mock(self) -> Box<dyn RgbDevice> {
        match self {
            Slot::Keyboard => Box::new(mock::mock_gmmk()),
            Slot::Motherboard => Box::new(mock::mock_msi_z890()),
            Slot::Gpu => Box::new(mock::mock_rtx5080()),
        }
    }
}

pub struct Entry {
    slot: Slot,
    real: bool,
    dev: Box<dyn RgbDevice>,
}

pub struct DeviceManager {
    devices: Vec<Entry>,
}

impl DeviceManager {
    pub fn new() -> Self {
        DeviceManager {
            devices: Self::probe_all(),
        }
    }

    /// Probe one slot's real backend. `None` = not present (or probe failed;
    /// logged). Slow — call without holding the manager lock.
    pub fn probe_real(slot: Slot) -> Option<Box<dyn RgbDevice>> {
        let (name, res): (&str, Result<Option<Box<dyn RgbDevice>>, _>) = match slot {
            Slot::Keyboard => (
                "gmmk",
                gmmk::probe().map(|o| o.map(|d| Box::new(d) as Box<dyn RgbDevice>)),
            ),
            Slot::Motherboard => (
                "msi",
                msi::probe().map(|o| o.map(|d| Box::new(d) as Box<dyn RgbDevice>)),
            ),
            #[cfg(windows)]
            Slot::Gpu => (
                "gigabyte-gpu",
                super::gigabyte_gpu::probe().map(|o| o.map(|d| Box::new(d) as Box<dyn RgbDevice>)),
            ),
            #[cfg(not(windows))]
            Slot::Gpu => ("gigabyte-gpu", Ok(None)),
        };
        match res {
            Ok(Some(dev)) => {
                crate::klog!("{name}: detected ({})", dev.id());
                Some(dev)
            }
            Ok(None) => None,
            Err(e) => {
                crate::klog!("{name}: probe failed ({e})");
                None
            }
        }
    }

    /// Probe every slot (real backend, else mock). Slow — no lock needed.
    pub fn probe_all() -> Vec<Entry> {
        Slot::ALL
            .iter()
            .map(|&slot| match Self::probe_real(slot) {
                Some(dev) => Entry {
                    slot,
                    real: true,
                    dev,
                },
                None => {
                    crate::klog!("{slot:?}: no hardware, using mock");
                    Entry {
                        slot,
                        real: false,
                        dev: slot.mock(),
                    }
                }
            })
            .collect()
    }

    /// Remove every device (for a full user-requested rescan). Drop the result
    /// *outside* the manager lock: device drops join their writer threads.
    pub fn take_all(&mut self) -> Vec<Entry> {
        std::mem::take(&mut self.devices)
    }

    pub fn set_all(&mut self, devices: Vec<Entry>) {
        self.devices = devices;
    }

    /// Slots currently served by a mock.
    pub fn missing_slots(&self) -> Vec<Slot> {
        self.devices
            .iter()
            .filter(|e| !e.real)
            .map(|e| e.slot)
            .collect()
    }

    /// Swap a mock slot for freshly probed real hardware. Returns whatever must
    /// be dropped (the replaced mock, or the newcomer if the slot is already
    /// real) — drop it outside the lock. Never replaces a real device.
    pub fn upgrade(&mut self, slot: Slot, dev: Box<dyn RgbDevice>) -> Option<Box<dyn RgbDevice>> {
        let Some(entry) = self.devices.iter_mut().find(|e| e.slot == slot) else {
            return Some(dev);
        };
        if entry.real {
            return Some(dev); // raced with another upgrade; discard the newcomer
        }
        entry.real = true;
        Some(std::mem::replace(&mut entry.dev, dev))
    }

    pub fn infos(&self) -> Vec<DeviceInfo> {
        self.devices.iter().map(|e| e.dev.info().clone()).collect()
    }

    /// Ids of devices backed by real hardware right now.
    pub fn real_ids(&self) -> Vec<DeviceId> {
        self.devices
            .iter()
            .filter(|e| e.real)
            .map(|e| e.dev.id().to_string())
            .collect()
    }

    pub fn get_mut(&mut self, id: &DeviceId) -> Option<&mut Box<dyn RgbDevice>> {
        self.devices
            .iter_mut()
            .map(|e| &mut e.dev)
            .find(|d| d.id() == id)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn RgbDevice>> {
        self.devices.iter_mut().map(|e| &mut e.dev)
    }

    /// Ask every device to re-assert its state on the next write.
    pub fn resync_all(&mut self) {
        for e in &mut self.devices {
            e.dev.resync();
        }
    }
}
