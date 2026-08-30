#![cfg_attr(not(test), no_std)]
#![deny(missing_docs)]

//! # `rusty_vulkan`
//!
//! A `#![no_std]` + `alloc` sovereign raw Vulkan hardware command buffer and
//! surface layer for the **Rusty Mill** ecosystem: a real dynamically-loaded
//! Vulkan 1.0 ICD binding (`vulkan-1.dll` on Windows, resolved via
//! `vkGetInstanceProcAddr` the way every Vulkan loader works — never a
//! static link-time dependency), not a stub.
//!
//! Previously, [`Instance::new`] never touched Vulkan at all (`handle` was
//! always a null pointer) and [`PhysicalDevice::new`] just wrapped whatever
//! `Instance` it was handed with no enumeration. Now: [`Instance::new`]
//! loads the real driver, creates a real `VkInstance`, and
//! [`Instance::enumerate_physical_devices`] calls the real
//! `vkEnumeratePhysicalDevices`, returning a [`PhysicalDevice`] per GPU
//! actually present, each of which reports its real
//! [`PhysicalDevice::properties`] (name, vendor/device ID, type) and real
//! [`PhysicalDevice::queue_families`] (queue counts and
//! graphics/compute/transfer/sparse-binding capability flags).
//!
//! Windows-only for now — see "Known gaps" below.

extern crate alloc;

#[cfg(windows)]
mod ffi;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows as platform;

use alloc::string::String;
use alloc::vec::Vec;
use core::fmt;

/// Errors from creating an instance or querying a physical device.
#[derive(Debug)]
pub enum VulkanError {
    /// This platform has no Vulkan loader binding in this crate yet (see
    /// "Known gaps" in the crate docs).
    UnsupportedPlatform,
    /// The Vulkan loader library (`vulkan-1.dll` on Windows) could not be
    /// loaded — no Vulkan-capable driver is installed.
    LoaderNotFound,
    /// A required entry point wasn't exported by the loader.
    MissingEntryPoint(&'static str),
    /// `vkCreateInstance` returned a non-success `VkResult`.
    CreateInstanceFailed(i32),
    /// `vkEnumeratePhysicalDevices` returned a non-success `VkResult`.
    EnumeratePhysicalDevicesFailed(i32),
}

impl fmt::Display for VulkanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VulkanError::UnsupportedPlatform => write!(f, "no Vulkan loader binding for this platform"),
            VulkanError::LoaderNotFound => write!(f, "could not load the Vulkan loader library"),
            VulkanError::MissingEntryPoint(name) => write!(f, "Vulkan loader is missing entry point {name}"),
            VulkanError::CreateInstanceFailed(code) => write!(f, "vkCreateInstance failed: VkResult {code}"),
            VulkanError::EnumeratePhysicalDevicesFailed(code) => {
                write!(f, "vkEnumeratePhysicalDevices failed: VkResult {code}")
            }
        }
    }
}

/// The kind of GPU a [`PhysicalDevice`] reports itself as (`VkPhysicalDeviceType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeviceType {
    /// A GPU embedded in or tightly coupled with the host.
    IntegratedGpu,
    /// A separate, typically much more powerful, GPU.
    DiscreteGpu,
    /// A virtualized GPU running in a virtual machine.
    VirtualGpu,
    /// No separate GPU; Vulkan implemented purely on the CPU.
    Cpu,
    /// A device type not covered above, or not fitting the other categories.
    Other,
    /// A raw `VkPhysicalDeviceType` value not recognized here (a newer
    /// Vulkan revision defined it after this crate was written).
    Unknown(u32),
}

impl From<u32> for DeviceType {
    fn from(value: u32) -> Self {
        match value {
            1 => DeviceType::IntegratedGpu,
            2 => DeviceType::DiscreteGpu,
            3 => DeviceType::VirtualGpu,
            4 => DeviceType::Cpu,
            0 => DeviceType::Other,
            other => DeviceType::Unknown(other),
        }
    }
}

/// A physical device's identifying properties (`VkPhysicalDeviceProperties`,
/// leading fields only — see [`ffi::physical_device_properties_offsets`]).
#[derive(Debug, Clone)]
pub struct PhysicalDeviceProperties {
    /// The Vulkan API version this device supports, packed per
    /// `VK_MAKE_API_VERSION`.
    pub api_version: u32,
    /// The driver's own version number (driver-defined encoding).
    pub driver_version: u32,
    /// The PCI vendor ID (or platform-defined ID for non-PCI devices).
    pub vendor_id: u32,
    /// The PCI device ID (or platform-defined ID).
    pub device_id: u32,
    /// The kind of device this is.
    pub device_type: DeviceType,
    /// The driver-reported device name (e.g. a real GPU model string).
    pub device_name: String,
}

/// A single queue family a [`PhysicalDevice`] exposes
/// (`VkQueueFamilyProperties`).
#[derive(Debug, Clone, Copy)]
pub struct QueueFamilyProperties {
    /// Number of queues in this family.
    pub queue_count: u32,
    /// Number of bits of timestamp value that are meaningful.
    pub timestamp_valid_bits: u32,
    // Never constructed off Windows (see `PhysicalDevice::queue_families`),
    // so keeping the field itself Windows-only avoids a dead-code warning
    // rather than papering over it with an `#[allow]`.
    #[cfg(windows)]
    raw_flags: u32,
}

#[cfg(windows)]
impl QueueFamilyProperties {
    /// Whether this family supports graphics operations.
    pub fn supports_graphics(&self) -> bool {
        self.raw_flags & ffi_consts::GRAPHICS != 0
    }
    /// Whether this family supports compute operations.
    pub fn supports_compute(&self) -> bool {
        self.raw_flags & ffi_consts::COMPUTE != 0
    }
    /// Whether this family supports transfer (copy) operations.
    pub fn supports_transfer(&self) -> bool {
        self.raw_flags & ffi_consts::TRANSFER != 0
    }
    /// Whether this family supports sparse memory binding operations.
    pub fn supports_sparse_binding(&self) -> bool {
        self.raw_flags & ffi_consts::SPARSE_BINDING != 0
    }
}

// Off Windows this crate has no real driver binding at all (see "Known
// gaps" above), so `raw_flags` is never populated with meaningful bits --
// every capability query honestly reports `false` rather than
// masking against a fake all-zero flag set.
#[cfg(not(windows))]
impl QueueFamilyProperties {
    /// Whether this family supports graphics operations.
    pub fn supports_graphics(&self) -> bool {
        false
    }
    /// Whether this family supports compute operations.
    pub fn supports_compute(&self) -> bool {
        false
    }
    /// Whether this family supports transfer (copy) operations.
    pub fn supports_transfer(&self) -> bool {
        false
    }
    /// Whether this family supports sparse memory binding operations.
    pub fn supports_sparse_binding(&self) -> bool {
        false
    }
}

#[cfg(windows)]
mod ffi_consts {
    pub const GRAPHICS: u32 = crate::ffi::VK_QUEUE_GRAPHICS_BIT;
    pub const COMPUTE: u32 = crate::ffi::VK_QUEUE_COMPUTE_BIT;
    pub const TRANSFER: u32 = crate::ffi::VK_QUEUE_TRANSFER_BIT;
    pub const SPARSE_BINDING: u32 = crate::ffi::VK_QUEUE_SPARSE_BINDING_BIT;
}

/// A real Vulkan instance: the loaded driver plus a live `VkInstance`
/// handle, torn down (`vkDestroyInstance` then unloading the driver
/// library) on drop.
pub struct Instance {
    #[cfg(windows)]
    inner: platform::InstanceInner,
    #[cfg(not(windows))]
    _unsupported: (),
}

impl Instance {
    /// Loads the platform's Vulkan driver and creates a real `VkInstance`.
    ///
    /// Fails with [`VulkanError::LoaderNotFound`] if no Vulkan-capable
    /// driver is installed, or [`VulkanError::CreateInstanceFailed`] if
    /// the driver rejects instance creation (e.g. no supported Vulkan
    /// version).
    pub fn new() -> Result<Self, VulkanError> {
        #[cfg(windows)]
        {
            Ok(Instance { inner: platform::InstanceInner::new()? })
        }
        #[cfg(not(windows))]
        {
            Err(VulkanError::UnsupportedPlatform)
        }
    }

    /// Enumerates the real physical GPU devices Vulkan reports on this
    /// instance (`vkEnumeratePhysicalDevices`).
    pub fn enumerate_physical_devices(&self) -> Result<Vec<PhysicalDevice>, VulkanError> {
        #[cfg(windows)]
        {
            self.inner.enumerate_physical_devices()
        }
        #[cfg(not(windows))]
        {
            Err(VulkanError::UnsupportedPlatform)
        }
    }
}

/// A single physical GPU device enumerated from an [`Instance`].
pub struct PhysicalDevice {
    #[cfg(windows)]
    inner: platform::PhysicalDeviceInner,
    #[cfg(not(windows))]
    _unsupported: (),
}

impl PhysicalDevice {
    /// This device's identifying properties (name, vendor/device ID, type).
    pub fn properties(&self) -> PhysicalDeviceProperties {
        #[cfg(windows)]
        {
            self.inner.properties()
        }
        #[cfg(not(windows))]
        {
            unreachable!("PhysicalDevice is never constructed on unsupported platforms")
        }
    }

    /// This device's queue families and their capabilities.
    pub fn queue_families(&self) -> Vec<QueueFamilyProperties> {
        #[cfg(windows)]
        {
            self.inner.queue_families()
        }
        #[cfg(not(windows))]
        {
            unreachable!("PhysicalDevice is never constructed on unsupported platforms")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // These exercise a real Vulkan driver where one is installed, and
    // skip (rather than fail) where none is — the same pattern used
    // elsewhere in this ecosystem (rusty_audio's WASAPI tests,
    // rusty_font's system-font tests) for hardware/driver-dependent
    // checks that can't assume a fixed CI environment.

    #[test]
    fn creating_an_instance_succeeds_when_a_driver_is_present() {
        match Instance::new() {
            Ok(_instance) => {}
            // Both variants mean "no usable Vulkan on this machine" -- the
            // former on Windows without a driver installed, the latter on
            // any platform this crate doesn't bind to at all (e.g. the
            // ubuntu-latest/macos-latest CI runners).
            Err(VulkanError::LoaderNotFound | VulkanError::UnsupportedPlatform) => {
                eprintln!("skipping: no Vulkan loader installed on this machine");
            }
            Err(e) => panic!("unexpected error creating instance: {e}"),
        }
    }

    #[test]
    fn enumerating_physical_devices_finds_at_least_one_real_gpu() {
        let Ok(instance) = Instance::new() else {
            eprintln!("skipping: no Vulkan loader installed on this machine");
            return;
        };
        let devices = instance.enumerate_physical_devices().unwrap();
        assert!(!devices.is_empty(), "expected at least one physical device");
    }

    #[test]
    fn a_real_physical_device_reports_a_nonempty_name_and_a_recognized_type() {
        let Ok(instance) = Instance::new() else {
            eprintln!("skipping: no Vulkan loader installed on this machine");
            return;
        };
        let devices = instance.enumerate_physical_devices().unwrap();
        let Some(device) = devices.first() else {
            eprintln!("skipping: no physical devices reported");
            return;
        };
        let props = device.properties();
        assert!(!props.device_name.is_empty(), "device name should be non-empty");
        assert!(
            !matches!(props.device_type, DeviceType::Unknown(_)),
            "device type should be a recognized VkPhysicalDeviceType, got {:?}",
            props.device_type
        );
    }

    #[test]
    fn a_real_physical_device_reports_at_least_one_queue_family_with_queues() {
        let Ok(instance) = Instance::new() else {
            eprintln!("skipping: no Vulkan loader installed on this machine");
            return;
        };
        let devices = instance.enumerate_physical_devices().unwrap();
        let Some(device) = devices.first() else {
            eprintln!("skipping: no physical devices reported");
            return;
        };
        let families = device.queue_families();
        assert!(!families.is_empty(), "expected at least one queue family");
        assert!(families.iter().any(|f| f.queue_count > 0));
    }
}
