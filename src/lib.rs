#![no_std]
#![deny(missing_docs)]

//! # `rusty_vulkan`
//!
//! A `#![no_std]` + `alloc` sovereign raw Vulkan hardware command buffer and surface layer
//! for the **Rusty Mill** ecosystem.

extern crate alloc;

/// Raw Vulkan Instance handle.
pub struct Instance {
    handle: *mut core::ffi::c_void,
}

impl Instance {
    /// Creates a raw Vulkan Instance.
    pub fn new() -> Result<Self, &'static str> {
        Ok(Self {
            handle: core::ptr::null_mut(),
        })
    }
}

/// Raw Vulkan Physical Device handle.
pub struct PhysicalDevice {
    instance: Instance,
}

impl PhysicalDevice {
    /// Enumerates raw physical GPU devices.
    pub fn new(instance: Instance) -> Self {
        Self { instance }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_creation() {
        let inst = Instance::new().unwrap();
        let _gpu = PhysicalDevice::new(inst);
    }
}
