//! The Windows Vulkan loader binding: `vulkan-1.dll`, resolved and called
//! through [`rusty_win32::dynlib`] plus this crate's own [`crate::ffi`]
//! ABI definitions.

use alloc::string::String;
use alloc::vec::Vec;
use core::ffi::CStr;
use core::mem::transmute;
use core::ptr::null_mut;

use rusty_win32::RawHandle;
use rusty_win32::dynlib::{free_library, get_proc_address, load_library};

use crate::ffi::{
    self, PfnCreateInstance, PfnDestroyInstance, PfnEnumeratePhysicalDevices,
    PfnGetInstanceProcAddr, PfnGetPhysicalDeviceProperties,
    PfnGetPhysicalDeviceQueueFamilyProperties, VK_SUCCESS, VkApplicationInfo, VkInstance,
    VkInstanceCreateInfo, VkPhysicalDevice, VkQueueFamilyProperties,
};
use crate::{DeviceType, PhysicalDeviceProperties, QueueFamilyProperties, VulkanError};

/// Resolves `name` as an instance-level Vulkan function via
/// `vkGetInstanceProcAddr`, transmuting it to the given function-pointer
/// type. `instance` may be null only for the handful of functions Vulkan
/// defines as instance-independent (e.g. `vkCreateInstance`).
///
/// # Safety
///
/// `T` must be exactly the ABI signature Vulkan documents for `name`; a
/// mismatch is instant undefined behavior on first call.
unsafe fn resolve<T: Copy>(
    get_instance_proc_addr: PfnGetInstanceProcAddr,
    instance: VkInstance,
    name: &'static CStr,
) -> Result<T, VulkanError> {
    // SAFETY: `get_instance_proc_addr` was itself resolved from the loaded
    // driver and is being called with a null-terminated name, per Vulkan's
    // own contract for this function.
    let raw = unsafe { get_instance_proc_addr(instance, name.as_ptr()) };
    let raw = raw.ok_or_else(|| {
        VulkanError::MissingEntryPoint(
            // SAFETY: every name this function is called with below is a
            // 'static byte-string literal.
            unsafe { core::str::from_utf8_unchecked(name.to_bytes()) },
        )
    })?;
    // SAFETY: caller guarantees `T` matches `name`'s real Vulkan ABI (a
    // bare `unsafe extern "system" fn(...)`, always pointer-sized —
    // `transmute_copy` is used instead of `transmute` only because `T`'s
    // size isn't known to the compiler at this generic call site).
    Ok(unsafe { core::mem::transmute_copy::<ffi::PfnVoidFunction, T>(&raw) })
}

struct InstanceFns {
    destroy_instance: PfnDestroyInstance,
    enumerate_physical_devices: PfnEnumeratePhysicalDevices,
    get_physical_device_properties: PfnGetPhysicalDeviceProperties,
    get_physical_device_queue_family_properties: PfnGetPhysicalDeviceQueueFamilyProperties,
}

pub struct InstanceInner {
    module: RawHandle,
    instance: VkInstance,
    fns: InstanceFns,
}

// SAFETY: `module`/`instance` are stable driver-owned handles; nothing
// here is thread-affine (Vulkan itself is designed for multi-threaded use
// with external synchronization, which this crate's `&self`/`&mut self`
// borrow rules already provide).
unsafe impl Send for InstanceInner {}
unsafe impl Sync for InstanceInner {}

impl InstanceInner {
    pub fn new() -> Result<Self, VulkanError> {
        let module = load_library("vulkan-1.dll").map_err(|_| VulkanError::LoaderNotFound)?;

        // SAFETY: `module` was just loaded above and is freed only in
        // `Drop`, after every use of `get_instance_proc_addr`/`fns`.
        let get_instance_proc_addr_raw =
            unsafe { get_proc_address(module, "vkGetInstanceProcAddr") };
        let Some(get_instance_proc_addr_raw) = get_instance_proc_addr_raw else {
            // SAFETY: `module` isn't used again after this.
            unsafe {
                let _ = free_library(module);
            }
            return Err(VulkanError::MissingEntryPoint("vkGetInstanceProcAddr"));
        };
        // SAFETY: `vkGetInstanceProcAddr` has this exact signature per the
        // Vulkan spec; `get_instance_proc_addr_raw` was resolved from the
        // real loaded driver.
        let get_instance_proc_addr: PfnGetInstanceProcAddr =
            unsafe { transmute(get_instance_proc_addr_raw) };

        let create_instance_result: Result<PfnCreateInstance, VulkanError> =
            unsafe { resolve(get_instance_proc_addr, null_mut(), c"vkCreateInstance") };
        let create_instance = match create_instance_result {
            Ok(f) => f,
            Err(e) => {
                unsafe {
                    let _ = free_library(module);
                }
                return Err(e);
            }
        };

        let app_info = VkApplicationInfo::new(c"rusty_vulkan", c"rusty_vulkan");
        let create_info = VkInstanceCreateInfo::new(&app_info);
        let mut instance: VkInstance = null_mut();
        // SAFETY: `create_info` is fully initialized and its
        // `p_application_info` points at `app_info`, which outlives this
        // call; `instance` is a valid out-pointer.
        let result = unsafe { create_instance(&create_info, core::ptr::null(), &mut instance) };
        if result != VK_SUCCESS {
            unsafe {
                let _ = free_library(module);
            }
            return Err(VulkanError::CreateInstanceFailed(result));
        }

        let fns = match Self::resolve_instance_fns(get_instance_proc_addr, instance) {
            Ok(fns) => fns,
            Err(e) => {
                unsafe {
                    let _ = free_library(module);
                }
                return Err(e);
            }
        };

        Ok(InstanceInner {
            module,
            instance,
            fns,
        })
    }

    fn resolve_instance_fns(
        get_instance_proc_addr: PfnGetInstanceProcAddr,
        instance: VkInstance,
    ) -> Result<InstanceFns, VulkanError> {
        // SAFETY: `instance` is the just-created, still-live `VkInstance`
        // from `new`; each name/type pair below matches Vulkan's
        // documented ABI for that function.
        unsafe {
            Ok(InstanceFns {
                destroy_instance: resolve(get_instance_proc_addr, instance, c"vkDestroyInstance")?,
                enumerate_physical_devices: resolve(
                    get_instance_proc_addr,
                    instance,
                    c"vkEnumeratePhysicalDevices",
                )?,
                get_physical_device_properties: resolve(
                    get_instance_proc_addr,
                    instance,
                    c"vkGetPhysicalDeviceProperties",
                )?,
                get_physical_device_queue_family_properties: resolve(
                    get_instance_proc_addr,
                    instance,
                    c"vkGetPhysicalDeviceQueueFamilyProperties",
                )?,
            })
        }
    }

    pub fn enumerate_physical_devices(&self) -> Result<Vec<crate::PhysicalDevice>, VulkanError> {
        let mut count: u32 = 0;
        // SAFETY: `self.instance` is live; passing a null device buffer
        // with a valid `count` out-pointer is the documented two-call
        // Vulkan enumeration idiom.
        let result =
            unsafe { (self.fns.enumerate_physical_devices)(self.instance, &mut count, null_mut()) };
        if result != VK_SUCCESS {
            return Err(VulkanError::EnumeratePhysicalDevicesFailed(result));
        }

        let mut devices: Vec<VkPhysicalDevice> = alloc::vec![null_mut(); count as usize];
        if count > 0 {
            // SAFETY: `devices` has exactly `count` elements, matching
            // what was just reported.
            let result = unsafe {
                (self.fns.enumerate_physical_devices)(
                    self.instance,
                    &mut count,
                    devices.as_mut_ptr(),
                )
            };
            if result != VK_SUCCESS {
                return Err(VulkanError::EnumeratePhysicalDevicesFailed(result));
            }
        }

        Ok(devices
            .into_iter()
            .map(|handle| crate::PhysicalDevice {
                inner: PhysicalDeviceInner {
                    handle,
                    get_properties: self.fns.get_physical_device_properties,
                    get_queue_family_properties: self
                        .fns
                        .get_physical_device_queue_family_properties,
                },
            })
            .collect())
    }
}

impl Drop for InstanceInner {
    fn drop(&mut self) {
        // SAFETY: `self.instance` is live until this point and never used
        // again after; `self.module` is freed only after `vkDestroyInstance`
        // returns.
        unsafe {
            (self.fns.destroy_instance)(self.instance, core::ptr::null());
            let _ = free_library(self.module);
        }
    }
}

pub struct PhysicalDeviceInner {
    handle: VkPhysicalDevice,
    get_properties: PfnGetPhysicalDeviceProperties,
    get_queue_family_properties: PfnGetPhysicalDeviceQueueFamilyProperties,
}

impl PhysicalDeviceInner {
    pub fn properties(&self) -> PhysicalDeviceProperties {
        use ffi::physical_device_properties_offsets as off;

        let mut buf: Vec<u64> = alloc::vec![0u64; ffi::PHYSICAL_DEVICE_PROPERTIES_BUFFER_U64_LEN];
        // SAFETY: `self.handle` is a live physical device from the same
        // instance `get_properties` was resolved on; `buf` is large enough
        // for the real `VkPhysicalDeviceProperties`.
        unsafe { (self.get_properties)(self.handle, buf.as_mut_ptr() as *mut u8) };
        let base = buf.as_ptr() as *const u8;

        // SAFETY: each read is within `buf`'s allocated length, computed
        // from the fixed offsets `VkPhysicalDeviceProperties` documents;
        // `read_unaligned` tolerates any offset regardless of the field's
        // natural alignment.
        let (api_version, driver_version, vendor_id, device_id, device_type_raw) = unsafe {
            (
                core::ptr::read_unaligned(base.add(off::API_VERSION) as *const u32),
                core::ptr::read_unaligned(base.add(off::DRIVER_VERSION) as *const u32),
                core::ptr::read_unaligned(base.add(off::VENDOR_ID) as *const u32),
                core::ptr::read_unaligned(base.add(off::DEVICE_ID) as *const u32),
                core::ptr::read_unaligned(base.add(off::DEVICE_TYPE) as *const u32),
            )
        };

        // SAFETY: `DEVICE_NAME_LEN` bytes starting at `DEVICE_NAME` are
        // within `buf`.
        let name_bytes = unsafe {
            core::slice::from_raw_parts(base.add(off::DEVICE_NAME), off::DEVICE_NAME_LEN)
        };
        let nul = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(name_bytes.len());
        let device_name = String::from_utf8_lossy(&name_bytes[..nul]).into_owned();

        PhysicalDeviceProperties {
            api_version,
            driver_version,
            vendor_id,
            device_id,
            device_type: DeviceType::from(device_type_raw),
            device_name,
        }
    }

    pub fn queue_families(&self) -> Vec<QueueFamilyProperties> {
        let mut count: u32 = 0;
        // SAFETY: `self.handle` is live; a null properties buffer with a
        // valid `count` out-pointer is the documented two-call idiom.
        unsafe { (self.get_queue_family_properties)(self.handle, &mut count, null_mut()) };

        let mut raw: Vec<VkQueueFamilyProperties> =
            alloc::vec![VkQueueFamilyProperties::default(); count as usize];
        if count > 0 {
            // SAFETY: `raw` has exactly `count` elements, matching what
            // was just reported.
            unsafe {
                (self.get_queue_family_properties)(self.handle, &mut count, raw.as_mut_ptr())
            };
        }

        raw.into_iter()
            .map(|f| QueueFamilyProperties {
                queue_count: f.queue_count,
                timestamp_valid_bits: f.timestamp_valid_bits,
                raw_flags: f.queue_flags,
            })
            .collect()
    }
}
