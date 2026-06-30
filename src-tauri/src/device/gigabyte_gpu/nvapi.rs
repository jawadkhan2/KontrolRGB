//! Minimal NvAPI FFI for GPU I2C access (Windows only).
//!
//! Gigabyte's GPU RGB controller sits on the card's I2C **port 1**. On Windows
//! the only way to reach that bus is NVIDIA's closed `nvapi64.dll`, which
//! exports a single resolver, `nvapi_QueryInterface(id) -> fn_ptr`; every other
//! entry point is fetched by a magic 32-bit interface id. We dynamically load
//! the DLL (like the WinRing0 fan driver) so the app still starts on machines
//! without an NVIDIA driver.
//!
//! Only the five functions needed for RGB are resolved: Initialize, enumerate
//! physical GPUs, read PCI ids (to confirm the card is a Gigabyte board), and
//! the I2C read/write pair. The interface ids and the `NV_I2C_INFO_V3` layout
//! are public hardware-interop facts (same values OpenRGB and NVFC use).

#![allow(non_snake_case)]

use std::ffi::c_void;
use std::sync::Arc;

use libloading::Library;

use crate::device::DeviceError;

/// Opaque NvAPI physical-GPU handle. NvAPI hands these back as pointers; we
/// only ever pass them straight back, so we keep them as `usize` to stay Send.
pub type GpuHandle = usize;

const NVAPI_MAX_PHYSICAL_GPUS: usize = 64;

// QueryInterface ids (stable across driver versions).
const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_UNLOAD: u32 = 0xD22B_DD7E;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GPU_GET_PCI_IDENTIFIERS: u32 = 0x2DDF_B66E;
const ID_I2C_WRITE_EX: u32 = 0x283A_C65A;
const ID_I2C_READ_EX: u32 = 0x4D7B_0709;

/// `NV_I2C_SPEED::NVAPI_I2C_SPEED_DEFAULT` (let the driver pick).
const NV_I2C_SPEED_DEFAULT: u32 = 0;
/// Deprecated speed field; NvAPI expects this sentinel.
const NV_I2C_SPEED_DEPRECATED: u32 = 0xFFFF;

/// `NV_I2C_INFO` version 3 — exact field order/types matter, the `version`
/// field is checksummed as `(3 << 16) | sizeof(struct)`.
#[repr(C)]
struct NvI2cInfoV3 {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    i2c_dev_address: u8,
    i2c_reg_address: *mut u8,
    reg_addr_size: u32,
    data: *mut u8,
    size: u32,
    i2c_speed: u32,
    i2c_speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

impl NvI2cInfoV3 {
    fn version() -> u32 {
        (3u32 << 16) | (std::mem::size_of::<NvI2cInfoV3>() as u32)
    }
}

// NvAPI uses the platform C ABI (Win64) for every resolved function.
type QueryInterfaceFn = unsafe extern "C" fn(u32) -> *const c_void;
type InitializeFn = unsafe extern "C" fn() -> i32;
type UnloadFn = unsafe extern "C" fn() -> i32;
type EnumPhysicalGpusFn = unsafe extern "C" fn(*mut usize, *mut i32) -> i32;
type GetPciIdentifiersFn =
    unsafe extern "C" fn(usize, *mut u32, *mut u32, *mut u32, *mut u32) -> i32;
type I2cExFn = unsafe extern "C" fn(usize, *mut NvI2cInfoV3, *mut u32) -> i32;

/// Resolved NvAPI entry points. The `Library` is held so the DLL stays mapped
/// for the lifetime of the resolved function pointers.
pub struct NvApi {
    _lib: Library,
    unload: UnloadFn,
    enum_physical_gpus: EnumPhysicalGpusFn,
    get_pci_identifiers: GetPciIdentifiersFn,
    i2c_write_ex: I2cExFn,
    i2c_read_ex: I2cExFn,
}

// All fields are either a Library (Send+Sync) or bare fn pointers (Send+Sync).
unsafe impl Send for NvApi {}
unsafe impl Sync for NvApi {}

fn comm(e: impl std::fmt::Display) -> DeviceError {
    DeviceError::Comm(format!("nvapi: {e}"))
}

impl NvApi {
    /// Load `nvapi64.dll`, resolve the entry points, and call Initialize.
    /// Returns `Ok(None)` when the DLL is absent (no NVIDIA driver) so the
    /// caller can fall back to the mock.
    pub fn load() -> Result<Option<Arc<NvApi>>, DeviceError> {
        let lib = match unsafe { Library::new("nvapi64.dll") } {
            Ok(lib) => lib,
            Err(_) => return Ok(None),
        };

        // SAFETY: nvapi64.dll exports `nvapi_QueryInterface`; every fn below is
        // resolved through it with NVIDIA's documented interface ids, and the
        // signatures match NvAPI's public ABI.
        unsafe {
            let query: QueryInterfaceFn = {
                let sym: libloading::Symbol<QueryInterfaceFn> =
                    lib.get(b"nvapi_QueryInterface\0").map_err(comm)?;
                *sym
            };

            let resolve = |id: u32| -> Result<*const c_void, DeviceError> {
                let p = query(id);
                if p.is_null() {
                    Err(comm(format!("QueryInterface(0x{id:08X}) returned null")))
                } else {
                    Ok(p)
                }
            };

            let initialize: InitializeFn = std::mem::transmute(resolve(ID_INITIALIZE)?);
            let unload: UnloadFn = std::mem::transmute(resolve(ID_UNLOAD)?);
            let enum_physical_gpus: EnumPhysicalGpusFn =
                std::mem::transmute(resolve(ID_ENUM_PHYSICAL_GPUS)?);
            let get_pci_identifiers: GetPciIdentifiersFn =
                std::mem::transmute(resolve(ID_GPU_GET_PCI_IDENTIFIERS)?);
            let i2c_write_ex: I2cExFn = std::mem::transmute(resolve(ID_I2C_WRITE_EX)?);
            let i2c_read_ex: I2cExFn = std::mem::transmute(resolve(ID_I2C_READ_EX)?);

            let status = initialize();
            if status != 0 {
                return Err(comm(format!("NvAPI_Initialize failed ({status})")));
            }

            Ok(Some(Arc::new(NvApi {
                _lib: lib,
                unload,
                enum_physical_gpus,
                get_pci_identifiers,
                i2c_write_ex,
                i2c_read_ex,
            })))
        }
    }

    /// Enumerate physical GPU handles.
    pub fn enum_physical_gpus(&self) -> Result<Vec<GpuHandle>, DeviceError> {
        let mut handles = [0usize; NVAPI_MAX_PHYSICAL_GPUS];
        let mut count: i32 = 0;
        // SAFETY: handles/count out-params sized per NvAPI's contract.
        let status =
            unsafe { (self.enum_physical_gpus)(handles.as_mut_ptr(), &mut count) };
        if status != 0 {
            return Err(comm(format!("NvAPI_EnumPhysicalGPUs failed ({status})")));
        }
        let count = count.clamp(0, NVAPI_MAX_PHYSICAL_GPUS as i32) as usize;
        Ok(handles[..count].to_vec())
    }

    /// Returns `(pci_vendor, pci_device, subsystem_vendor, subsystem_device)`.
    pub fn pci_identifiers(
        &self,
        handle: GpuHandle,
    ) -> Result<(u16, u16, u16, u16), DeviceError> {
        let mut device_id: u32 = 0;
        let mut subsystem_id: u32 = 0;
        let mut revision_id: u32 = 0;
        let mut ext_device_id: u32 = 0;
        // SAFETY: four u32 out-params per NvAPI's contract.
        let status = unsafe {
            (self.get_pci_identifiers)(
                handle,
                &mut device_id,
                &mut subsystem_id,
                &mut revision_id,
                &mut ext_device_id,
            )
        };
        if status != 0 {
            return Err(comm(format!("NvAPI_GPU_GetPCIIdentifiers failed ({status})")));
        }
        Ok((
            (device_id & 0xFFFF) as u16,
            (device_id >> 16) as u16,
            (subsystem_id & 0xFFFF) as u16,
            (subsystem_id >> 16) as u16,
        ))
    }

    fn make_info(addr: u8, buf: &mut [u8]) -> NvI2cInfoV3 {
        NvI2cInfoV3 {
            version: NvI2cInfoV3::version(),
            display_mask: 0,
            is_ddc_port: 0,
            i2c_dev_address: addr << 1, // NvAPI wants the 8-bit (shifted) address
            i2c_reg_address: std::ptr::null_mut(),
            reg_addr_size: 0,
            data: buf.as_mut_ptr(),
            size: buf.len() as u32,
            i2c_speed: NV_I2C_SPEED_DEPRECATED,
            i2c_speed_khz: NV_I2C_SPEED_DEFAULT,
            port_id: 1, // RGB controller lives on port 1
            is_port_id_set: 1,
        }
    }

    /// Raw I2C block write to `addr` on port 1 (no register address byte).
    pub fn i2c_write_block(&self, handle: GpuHandle, addr: u8, data: &[u8]) -> Result<(), DeviceError> {
        let mut buf = data.to_vec();
        let mut info = Self::make_info(addr, &mut buf);
        let mut unknown: u32 = 0;
        // SAFETY: `info` borrows `buf` for the duration of the call only.
        let status = unsafe { (self.i2c_write_ex)(handle, &mut info, &mut unknown) };
        if status != 0 {
            return Err(comm(format!("NvAPI_I2CWriteEx failed ({status})")));
        }
        Ok(())
    }

    /// Raw I2C block read of `len` bytes from `addr` on port 1.
    pub fn i2c_read_block(
        &self,
        handle: GpuHandle,
        addr: u8,
        len: usize,
    ) -> Result<Vec<u8>, DeviceError> {
        let mut buf = vec![0u8; len];
        let mut info = Self::make_info(addr, &mut buf);
        let mut unknown: u32 = 0;
        // SAFETY: `info` borrows `buf`, sized `len`, for the call only.
        let status = unsafe { (self.i2c_read_ex)(handle, &mut info, &mut unknown) };
        if status != 0 {
            return Err(comm(format!("NvAPI_I2CReadEx failed ({status})")));
        }
        let n = (info.size as usize).min(buf.len());
        buf.truncate(n);
        Ok(buf)
    }
}

impl Drop for NvApi {
    fn drop(&mut self) {
        // Balance NvAPI_Initialize so the driver's internal refcount stays
        // correct across re-detects (rescan drops and rebuilds this).
        // SAFETY: resolved NvAPI export, no args.
        unsafe {
            (self.unload)();
        }
    }
}
