use crate::cf::{self, CFRelease};
use crate::model::{Battery, Charger, Disk};
use std::time::Instant;

pub struct DiskSampler {
    previous_read: Option<u64>,
    previous_write: Option<u64>,
    previous: Option<Instant>,
}

impl DiskSampler {
    pub fn new() -> Self {
        Self {
            previous_read: None,
            previous_write: None,
            previous: None,
        }
    }

    pub fn sample(&mut self) -> Disk {
        let (free, total) = capacity();
        let (read, write) = throughput();
        let now = Instant::now();
        let rates = match (self.previous_read, self.previous_write, self.previous) {
            (Some(before_read), Some(before_write), Some(before)) => {
                let elapsed = now.duration_since(before).as_secs_f64();
                if elapsed > 0.0 {
                    (
                        Some(read.wrapping_sub(before_read) as f64 / elapsed),
                        Some(write.wrapping_sub(before_write) as f64 / elapsed),
                    )
                } else {
                    (None, None)
                }
            }
            _ => (None, None),
        };
        self.previous_read = Some(read);
        self.previous_write = Some(write);
        self.previous = Some(now);
        Disk {
            total_bytes: total,
            free_bytes: free,
            read_bytes_per_second: rates.0,
            write_bytes_per_second: rates.1,
        }
    }
}

fn capacity() -> (u64, u64) {
    let mut stats = unsafe { std::mem::zeroed::<libc::statfs>() };
    if unsafe { libc::statfs(c"/".as_ptr(), &mut stats) } != 0 {
        return (0, 0);
    }
    let block = stats.f_bsize as u64;
    (stats.f_bavail * block, stats.f_blocks * block)
}

fn throughput() -> (u64, u64) {
    let mut read = 0u64;
    let mut write = 0u64;
    for service in cf::services("IOBlockStorageDriver") {
        let key = cf::cf_string("Statistics");
        let stats =
            unsafe { cf::IORegistryEntryCreateCFProperty(service, key, std::ptr::null(), 0) };
        unsafe { CFRelease(key) };
        if !stats.is_null() {
            read += cf::number_f64(cf::dict_get(stats, "Bytes (read)")).unwrap_or(0.0) as u64;
            write += cf::number_f64(cf::dict_get(stats, "Bytes (write)")).unwrap_or(0.0) as u64;
            unsafe { CFRelease(stats) };
        }
        unsafe { cf::IOObjectRelease(service) };
    }
    (read, write)
}

#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    fn IOPSCopyPowerSourcesInfo() -> *const std::ffi::c_void;
    fn IOPSCopyPowerSourcesList(blob: *const std::ffi::c_void) -> *const std::ffi::c_void;
    fn IOPSGetPowerSourceDescription(
        blob: *const std::ffi::c_void,
        source: *const std::ffi::c_void,
    ) -> *const std::ffi::c_void;
}

pub fn battery() -> Option<Battery> {
    let blob = unsafe { IOPSCopyPowerSourcesInfo() };
    if blob.is_null() {
        return None;
    }
    let list = unsafe { IOPSCopyPowerSourcesList(blob) };
    if list.is_null() {
        unsafe { CFRelease(blob) };
        return None;
    }
    let mut found = None;
    let count = unsafe { cf::CFArrayGetCount(list) };
    for index in 0..count {
        let source = unsafe { cf::CFArrayGetValueAtIndex(list, index) };
        let description = unsafe { IOPSGetPowerSourceDescription(blob, source) };
        let flag = cf::dict_get(description, "Is Present");
        let present = cf::boolean(flag)
            .or_else(|| cf::number_f64(flag).map(|value| value > 0.0))
            .unwrap_or(false);
        if !present {
            continue;
        }
        let current = cf::number_f64(cf::dict_get(description, "Current Capacity")).unwrap_or(0.0);
        let max = cf::number_f64(cf::dict_get(description, "Max Capacity")).unwrap_or(100.0);
        let fraction = if max > 0.0 {
            (current / max).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let smart = smart_battery();
        let charging = cf::boolean(cf::dict_get(description, "Is Charging")).unwrap_or(false);
        let external_power = cf::string(cf::dict_get(description, "Power Source State"))
            .is_some_and(|state| state == "AC Power");
        let remaining_key = if charging {
            "Time to Full Charge"
        } else {
            "Time to Empty"
        };
        let minutes_remaining = cf::number_f64(cf::dict_get(description, remaining_key))
            .filter(|minutes| *minutes > 0.0)
            .map(|minutes| minutes as u32);
        found = Some(Battery {
            fraction,
            health: smart.health,
            cycles: smart.cycles,
            charging,
            external_power,
            minutes_remaining,
            battery_watts: smart.battery_watts,
            charger: if external_power { smart.charger } else { None },
        });
        break;
    }
    unsafe {
        CFRelease(list);
        CFRelease(blob);
    }
    found
}

#[derive(Default)]
struct SmartBattery {
    health: Option<f64>,
    cycles: Option<u32>,
    battery_watts: Option<f64>,
    charger: Option<Charger>,
}

fn smart_battery() -> SmartBattery {
    let services = cf::services("AppleSmartBattery");
    let Some(service) = services.first().copied() else {
        return SmartBattery::default();
    };
    let properties = cf::entry_properties(service);
    for extra in services {
        unsafe { cf::IOObjectRelease(extra) };
    }
    if properties.is_null() {
        return SmartBattery::default();
    }
    let number = |dict, key| cf::number_f64(cf::dict_get(dict, key));
    let design = number(properties, "DesignCapacity").unwrap_or(0.0);
    let raw = number(properties, "AppleRawMaxCapacity")
        .or_else(|| number(properties, "NominalChargeCapacity"))
        .unwrap_or(0.0);
    let health = if design > 0.0 && raw > 0.0 {
        Some((raw / design).clamp(0.0, 1.0))
    } else {
        None
    };
    let cycles = number(properties, "CycleCount").map(|value| value as u32);
    // Amperage is in mA and Voltage in mV, so the product is in µW.
    let battery_watts = match (
        number(properties, "InstantAmperage").or_else(|| number(properties, "Amperage")),
        number(properties, "Voltage"),
    ) {
        (Some(milliamps), Some(millivolts)) if millivolts > 0.0 => {
            Some(milliamps * millivolts / 1_000_000.0)
        }
        _ => None,
    };
    let adapter = cf::dict_get(properties, "AdapterDetails");
    let telemetry = cf::dict_get(properties, "PowerTelemetryData");
    let milli = |dict, key| {
        number(dict, key)
            .filter(|value| *value > 0.0)
            .map(|value| value / 1000.0)
    };
    let charger = (!adapter.is_null() || !telemetry.is_null()).then(|| Charger {
        rated_watts: number(adapter, "Watts").filter(|watts| *watts > 0.0),
        rated_volts: milli(adapter, "AdapterVoltage"),
        rated_amps: milli(adapter, "Current"),
        input_watts: milli(telemetry, "SystemPowerIn"),
        input_volts: milli(telemetry, "SystemVoltageIn"),
        input_amps: milli(telemetry, "SystemCurrentIn"),
        wireless: cf::boolean(cf::dict_get(adapter, "IsWireless")).unwrap_or(false),
    });
    unsafe { CFRelease(properties) };
    SmartBattery {
        health,
        cycles,
        battery_watts,
        charger,
    }
}
