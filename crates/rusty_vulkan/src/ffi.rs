//! Raw Vulkan 1.0 ABI: opaque handle types, the handful of `Vk*` structs
//! this crate builds or reads, and the function-pointer signatures
//! resolved dynamically through `vkGetInstanceProcAddr` (never a static
//! `#[link]` import — Vulkan is always a runtime-loaded ICD, not a
//! link-time dependency, per the spec's own loader model).

use core::ffi::{c_char, c_void};

/// Any Vulkan dispatchable or non-dispatchable handle is opaque to a
/// caller — just an address the driver interprets.
pub type VkInstance = *mut c_void;
/// See [`VkInstance`].
pub type VkPhysicalDevice = *mut c_void;

/// `VkResult`: `0` (`VK_SUCCESS`) or a positive/negative status code.
pub type VkResult = i32;
pub const VK_SUCCESS: VkResult = 0;

/// `VK_MAKE_API_VERSION(0, 1, 0, 0)` — requesting the Vulkan 1.0 core API.
pub const VK_API_VERSION_1_0: u32 = 1 << 22;

const VK_STRUCTURE_TYPE_APPLICATION_INFO: i32 = 0;
const VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO: i32 = 1;

/// `VkApplicationInfo` (Vulkan spec §4.1). Every field this crate sets is
/// either a fixed sentinel or a `'static` string, so the struct never
/// outlives the strings it points to.
#[repr(C)]
pub struct VkApplicationInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub p_application_name: *const c_char,
    pub application_version: u32,
    pub p_engine_name: *const c_char,
    pub engine_version: u32,
    pub api_version: u32,
}

impl VkApplicationInfo {
    pub fn new(app_name: &core::ffi::CStr, engine_name: &core::ffi::CStr) -> Self {
        VkApplicationInfo {
            s_type: VK_STRUCTURE_TYPE_APPLICATION_INFO,
            p_next: core::ptr::null(),
            p_application_name: app_name.as_ptr(),
            application_version: 0,
            p_engine_name: engine_name.as_ptr(),
            engine_version: 0,
            api_version: VK_API_VERSION_1_0,
        }
    }
}

/// `VkInstanceCreateInfo` (Vulkan spec §4.1). No layers/extensions are
/// requested — see the crate-level "Known gaps" doc.
#[repr(C)]
pub struct VkInstanceCreateInfo {
    pub s_type: i32,
    pub p_next: *const c_void,
    pub flags: u32,
    pub p_application_info: *const VkApplicationInfo,
    pub enabled_layer_count: u32,
    pub pp_enabled_layer_names: *const *const c_char,
    pub enabled_extension_count: u32,
    pub pp_enabled_extension_names: *const *const c_char,
}

impl VkInstanceCreateInfo {
    pub fn new(app_info: &VkApplicationInfo) -> Self {
        VkInstanceCreateInfo {
            s_type: VK_STRUCTURE_TYPE_INSTANCE_CREATE_INFO,
            p_next: core::ptr::null(),
            flags: 0,
            p_application_info: app_info,
            enabled_layer_count: 0,
            pp_enabled_layer_names: core::ptr::null(),
            enabled_extension_count: 0,
            pp_enabled_extension_names: core::ptr::null(),
        }
    }
}

/// `VkQueueFamilyProperties` (Vulkan spec §5.2) — the same layout on the
/// wire (four `u32`s, no padding), so this is read directly rather than
/// through a raw byte offset the way [`crate::PhysicalDeviceProperties`]
/// is.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VkQueueFamilyProperties {
    pub queue_flags: u32,
    pub queue_count: u32,
    pub timestamp_valid_bits: u32,
    pub min_image_transfer_granularity: [u32; 3],
}

pub const VK_QUEUE_GRAPHICS_BIT: u32 = 0x0000_0001;
pub const VK_QUEUE_COMPUTE_BIT: u32 = 0x0000_0002;
pub const VK_QUEUE_TRANSFER_BIT: u32 = 0x0000_0004;
pub const VK_QUEUE_SPARSE_BINDING_BIT: u32 = 0x0000_0008;

/// Byte offsets into `VkPhysicalDeviceProperties` (Vulkan spec §5.1) for
/// the leading fields this crate reads. The struct's full size (its
/// `VkPhysicalDeviceLimits`/`VkPhysicalDeviceSparseProperties` tail,
/// ~824 bytes on a 64-bit target) is deliberately not modeled field by
/// field — only these fixed, spec-guaranteed-stable leading offsets are
/// read, into a buffer generously over-sized for whatever trails them.
pub mod physical_device_properties_offsets {
    pub const API_VERSION: usize = 0;
    pub const DRIVER_VERSION: usize = 4;
    pub const VENDOR_ID: usize = 8;
    pub const DEVICE_ID: usize = 12;
    pub const DEVICE_TYPE: usize = 16;
    pub const DEVICE_NAME: usize = 20;
    pub const DEVICE_NAME_LEN: usize = 256; // VK_MAX_PHYSICAL_DEVICE_NAME_SIZE
}

/// A buffer large enough for the real `VkPhysicalDeviceProperties`
/// (~824 bytes on a 64-bit target) with headroom, expressed in `u64`
/// units so the allocation is naturally 8-byte aligned — the struct's
/// `VkPhysicalDeviceLimits` tail contains `VkDeviceSize` (`u64`) fields
/// that require it, even though this crate only reads the leading `u32`
/// fields directly.
pub const PHYSICAL_DEVICE_PROPERTIES_BUFFER_U64_LEN: usize = 256; // 2048 bytes

pub type PfnVoidFunction = unsafe extern "system" fn();
pub type PfnGetInstanceProcAddr =
    unsafe extern "system" fn(instance: VkInstance, name: *const c_char) -> Option<PfnVoidFunction>;
pub type PfnCreateInstance = unsafe extern "system" fn(
    create_info: *const VkInstanceCreateInfo,
    allocator: *const c_void,
    instance: *mut VkInstance,
) -> VkResult;
pub type PfnDestroyInstance = unsafe extern "system" fn(instance: VkInstance, allocator: *const c_void);
pub type PfnEnumeratePhysicalDevices = unsafe extern "system" fn(
    instance: VkInstance,
    count: *mut u32,
    devices: *mut VkPhysicalDevice,
) -> VkResult;
pub type PfnGetPhysicalDeviceProperties =
    unsafe extern "system" fn(device: VkPhysicalDevice, properties: *mut u8);
pub type PfnGetPhysicalDeviceQueueFamilyProperties = unsafe extern "system" fn(
    device: VkPhysicalDevice,
    count: *mut u32,
    properties: *mut VkQueueFamilyProperties,
);
