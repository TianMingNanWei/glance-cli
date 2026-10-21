use crate::cf::{self, CFRelease};
use crate::model::Gpu;

pub fn sample() -> Gpu {
    let mut best: Option<Gpu> = None;
    for service in cf::services("IOAccelerator") {
        let key = cf::cf_string("PerformanceStatistics");
        let stats =
            unsafe { cf::IORegistryEntryCreateCFProperty(service, key, std::ptr::null(), 0) };
        unsafe { CFRelease(key) };
        let gpu = if stats.is_null() {
            None
        } else {
            let utilization = dict_f64(
                stats,
                &[
                    "Device Utilization %",
                    "Renderer Utilization %",
                    "GPU Activity(%)",
                ],
            )
            .map(|value| {
                if value > 1.0 {
                    (value / 100.0).clamp(0.0, 1.0)
                } else {
                    value.clamp(0.0, 1.0)
                }
            });
            let memory = dict_f64(stats, &["In use system memory", "Alloc system memory"])
                .map(|value| value.max(0.0) as u64);
            unsafe { CFRelease(stats) };
            Some(Gpu {
                name: cf::entry_name(service),
                utilization,
                memory_bytes: memory,
                frequency_hz: None,
                power_watts: None,
                active: None,
            })
        };
        unsafe { cf::IOObjectRelease(service) };
        if let Some(gpu) = gpu.filter(|gpu| gpu.utilization.is_some() || gpu.memory_bytes.is_some())
        {
            let replace = best
                .as_ref()
                .map(|current| gpu.utilization.unwrap_or(0.0) > current.utilization.unwrap_or(0.0))
                .unwrap_or(true);
            if replace {
                best = Some(gpu);
            }
        }
    }
    best.unwrap_or(Gpu {
        name: String::new(),
        utilization: None,
        memory_bytes: None,
        frequency_hz: None,
        power_watts: None,
        active: None,
    })
}

fn dict_f64(dict: *const std::ffi::c_void, keys: &[&str]) -> Option<f64> {
    keys.iter()
        .find_map(|key| cf::number_f64(cf::dict_get(dict, key)))
}
