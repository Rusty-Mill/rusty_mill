//! Prints every real Vulkan physical device this machine's driver
//! reports, plus each device's real queue families — a visual sanity
//! check that `rusty_vulkan` talks to actual hardware, not a stub.

fn main() {
    let instance = rusty_vulkan::Instance::new().expect("failed to create Vulkan instance");
    let devices = instance
        .enumerate_physical_devices()
        .expect("failed to enumerate physical devices");

    println!("{} physical device(s):", devices.len());
    for device in &devices {
        let props = device.properties();
        println!(
            "- {} (vendor 0x{:04x}, device 0x{:04x}, type {:?}, api {}.{}.{})",
            props.device_name,
            props.vendor_id,
            props.device_id,
            props.device_type,
            props.api_version >> 22,
            (props.api_version >> 12) & 0x3ff,
            props.api_version & 0xfff,
        );
        for (i, family) in device.queue_families().iter().enumerate() {
            println!(
                "    queue family {i}: {} queue(s) [graphics={} compute={} transfer={} sparse_binding={}]",
                family.queue_count,
                family.supports_graphics(),
                family.supports_compute(),
                family.supports_transfer(),
                family.supports_sparse_binding(),
            );
        }
    }
}
