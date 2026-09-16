# rusty_vulkan

A `#![no_std]` + `alloc` sovereign raw Vulkan hardware command buffer and
GPU surface layer for the **Rusty Mill** ecosystem.

## What's real

Previously a total stub: `Instance::new` never touched Vulkan at all
(`handle` was always a null pointer) and `PhysicalDevice::new` just
wrapped whatever `Instance` it was handed, with no enumeration. All of
that is now real, on Windows:

- **`Instance::new`** dynamically loads `vulkan-1.dll` (via
  `rusty_win32::dynlib`, never a static link-time dependency — Vulkan is
  always a runtime-loaded ICD, per the spec's own loader model), resolves
  `vkGetInstanceProcAddr`, and calls the real `vkCreateInstance`.
- **`Instance::enumerate_physical_devices`** calls the real
  `vkEnumeratePhysicalDevices` (the documented two-call idiom: query the
  count, then fill a buffer of that size).
- **`PhysicalDevice::properties`** reads a real `vkGetPhysicalDeviceProperties`
  result — API/driver version, PCI vendor/device ID, device type, and the
  driver-reported device name string.
- **`PhysicalDevice::queue_families`** reads real
  `vkGetPhysicalDeviceQueueFamilyProperties` results, with
  `supports_graphics`/`supports_compute`/`supports_transfer`/
  `supports_sparse_binding` helpers over the raw capability bitmask.

**Verified against real hardware**, not synthetic data: `cargo test`
creates a real Vulkan instance and asserts on real enumerated devices
(non-empty name, a recognized device type, at least one queue family with
queues) where a driver is installed, skipping (not failing) otherwise.
`examples/list_devices.rs` prints every real physical device and its
queue families — on the machine this was built on, that's a real Intel
Arc iGPU and a real NVIDIA RTX 5070 Ti laptop GPU, each with correct,
distinct queue-family capabilities:

```
cargo run --example list_devices
```

## Known, deliberate gaps

- **Windows only.** The crate's description mentions Metal, but no macOS
  binding exists — `Instance::new` returns `VulkanError::UnsupportedPlatform`
  on any non-Windows target.
- **No logical device, command buffers, or surface/swapchain.** This
  crate currently covers instance creation and physical-device
  enumeration/querying only — the actual "hardware command buffer and GPU
  surface" layer its description promises isn't built yet.
- **No validation layers or extensions requested.** `vkCreateInstance` is
  called with an empty layer/extension list.
- **`VkPhysicalDeviceProperties`' `VkPhysicalDeviceLimits`/
  `VkPhysicalDeviceSparseProperties` tail isn't parsed** — only the
  leading fields (API/driver version, vendor/device ID, type, name) are
  read; the buffer passed to the driver is sized to hold the whole real
  struct, but the limits themselves aren't exposed yet.

## Testing

```
cargo test
cargo run --example list_devices
cargo clippy --all-targets
```
