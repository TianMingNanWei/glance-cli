//! Apple Silicon energy, P-state residency, and die temperature. None of this needs root.
use crate::cf::{self, CFRelease};
use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::time::Instant;

const ANE_ACTIVE_WATTS: f64 = 0.05;

#[derive(Clone, Debug, Default)]
pub struct SocReading {
    pub cpu_watts: Option<f64>,
    pub gpu_watts: Option<f64>,
    pub ane_watts: Option<f64>,
    pub dram_watts: Option<f64>,
    pub cluster_hz: HashMap<String, f64>,
    pub cluster_watts: HashMap<String, f64>,
    pub gpu_hz: Option<f64>,
    pub gpu_active: Option<f64>,
    pub temperature_c: Option<f64>,
}

impl SocReading {
    pub fn chip_watts(&self) -> Option<f64> {
        let parts: Vec<f64> = [
            self.cpu_watts,
            self.gpu_watts,
            self.ane_watts,
            self.dram_watts,
        ]
        .into_iter()
        .flatten()
        .collect();
        if parts.is_empty() {
            None
        } else {
            Some(parts.iter().sum())
        }
    }

    pub fn ane_active(&self) -> Option<bool> {
        self.ane_watts.map(|watts| watts >= ANE_ACTIVE_WATTS)
    }
}

pub struct SocMath;

impl SocMath {
    pub fn joules(value: i64, unit: &str) -> Option<f64> {
        let scale = match unit.trim() {
            "J" => 1.0,
            "mJ" => 1e-3,
            "uJ" | "µJ" => 1e-6,
            "nJ" => 1e-9,
            _ => return None,
        };
        Some(value as f64 * scale)
    }

    /// Frequencies in Hz from a pmgr `voltage-states` blob of (frequency, voltage) pairs.
    /// A leading zero is the off state. Values below 1e8 are kHz, which is how M4 and later
    /// publish CPU tables. A table of voltages alone is rejected.
    pub fn frequencies(data: &[u8]) -> Vec<f64> {
        let mut values = Vec::new();
        for chunk in data.as_chunks::<8>().0 {
            let frequency = u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            if frequency > 1 {
                values.push(frequency as f64);
            }
        }
        let Some(peak) = values.iter().copied().reduce(f64::max) else {
            return Vec::new();
        };
        let scale = if peak < 1e8 { 1_000.0 } else { 1.0 };
        let hertz: Vec<f64> = values.into_iter().map(|value| value * scale).collect();
        if hertz.iter().copied().fold(0.0, f64::max) >= 2e8 {
            hertz
        } else {
            Vec::new()
        }
    }

    pub fn residency(states: &[(String, i64)], table: &[f64]) -> (Option<f64>, f64) {
        let mut total = 0.0;
        let mut active = 0.0;
        let mut weighted = 0.0;
        let mut index = 0usize;
        for (name, residency) in states {
            let residency = (*residency).max(0) as f64;
            total += residency;
            if is_idle(name) {
                continue;
            }
            if index < table.len() {
                weighted += table[index] * residency;
            }
            active += residency;
            index += 1;
        }
        let hertz = if active > 0.0 && !table.is_empty() {
            Some(weighted / active)
        } else {
            None
        };
        (hertz, if total > 0.0 { active / total } else { 0.0 })
    }

    /// Cluster letter for channels such as "PCPU" or "MCPU1". Per-core channels are skipped.
    pub fn cluster_type(channel: &str) -> Option<String> {
        let mut chars = channel.chars();
        let letter = chars.next()?;
        let rest: String = chars.collect();
        if channel.contains('_') || !rest.starts_with("CPU") {
            return None;
        }
        let suffix = &rest[3..];
        if suffix.chars().all(|character| character.is_ascii_digit()) {
            Some(letter.to_string())
        } else {
            None
        }
    }
}

fn is_idle(name: &str) -> bool {
    matches!(name, "IDLE" | "DOWN" | "OFF")
}

struct ReportApi {
    copy: unsafe extern "C" fn(*const c_void, *const c_void, u64, u64, u64) -> *mut c_void,
    merge: unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void),
    subscribe: unsafe extern "C" fn(
        *const c_void,
        *mut c_void,
        *mut *mut c_void,
        u64,
        *const c_void,
    ) -> *mut c_void,
    samples: unsafe extern "C" fn(*mut c_void, *mut c_void, *const c_void) -> *mut c_void,
    delta: unsafe extern "C" fn(*const c_void, *const c_void, *const c_void) -> *mut c_void,
    group: unsafe extern "C" fn(*const c_void) -> *const c_void,
    name: unsafe extern "C" fn(*const c_void) -> *const c_void,
    unit: unsafe extern "C" fn(*const c_void) -> *const c_void,
    integer: unsafe extern "C" fn(*const c_void, i32) -> i64,
    state_count: unsafe extern "C" fn(*const c_void) -> i32,
    state_name: unsafe extern "C" fn(*const c_void, i32) -> *const c_void,
    state_residency: unsafe extern "C" fn(*const c_void, i32) -> i64,
}

impl ReportApi {
    fn load() -> Option<Self> {
        let library =
            unsafe { libc::dlopen(c"/usr/lib/libIOReport.dylib".as_ptr(), libc::RTLD_NOW) };
        if library.is_null() {
            return None;
        }
        unsafe fn symbol<T>(library: *mut c_void, name: &[u8]) -> Option<T> {
            let pointer = libc::dlsym(library, name.as_ptr().cast());
            if pointer.is_null() {
                None
            } else {
                Some(std::mem::transmute_copy(&pointer))
            }
        }
        Some(Self {
            copy: unsafe { symbol(library, b"IOReportCopyChannelsInGroup\0") }?,
            merge: unsafe { symbol(library, b"IOReportMergeChannels\0") }?,
            subscribe: unsafe { symbol(library, b"IOReportCreateSubscription\0") }?,
            samples: unsafe { symbol(library, b"IOReportCreateSamples\0") }?,
            delta: unsafe { symbol(library, b"IOReportCreateSamplesDelta\0") }?,
            group: unsafe { symbol(library, b"IOReportChannelGetGroup\0") }?,
            name: unsafe { symbol(library, b"IOReportChannelGetChannelName\0") }?,
            unit: unsafe { symbol(library, b"IOReportChannelGetUnitLabel\0") }?,
            integer: unsafe { symbol(library, b"IOReportSimpleGetIntegerValue\0") }?,
            state_count: unsafe { symbol(library, b"IOReportStateGetCount\0") }?,
            state_name: unsafe { symbol(library, b"IOReportStateGetNameForIndex\0") }?,
            state_residency: unsafe { symbol(library, b"IOReportStateGetResidency\0") }?,
        })
    }

    fn text(
        &self,
        function: unsafe extern "C" fn(*const c_void) -> *const c_void,
        channel: *const c_void,
    ) -> String {
        cf::string(unsafe { function(channel) }).unwrap_or_default()
    }
}

pub struct SocSampler {
    api: Option<ReportApi>,
    subscription: *mut c_void,
    subscribed: *mut c_void,
    previous: Option<(*const c_void, Instant)>,
    tables: HashMap<String, Vec<f64>>,
    thermal: Thermal,
}

impl SocSampler {
    pub fn new() -> Self {
        let api = ReportApi::load();
        let mut subscription = std::ptr::null_mut();
        let mut subscribed = std::ptr::null_mut();
        if let Some(api) = &api {
            if let Some(channels) = cf_string_copy(api, "Energy Model", None) {
                for (group, subgroup) in [
                    ("CPU Stats", "CPU Complex Performance States"),
                    ("GPU Stats", "GPU Performance States"),
                ] {
                    if let Some(more) = cf_string_copy(api, group, Some(subgroup)) {
                        unsafe { (api.merge)(channels, more, std::ptr::null()) };
                        unsafe { CFRelease(more) };
                    }
                }
                let mut out: *mut c_void = std::ptr::null_mut();
                subscription = unsafe {
                    (api.subscribe)(std::ptr::null(), channels, &mut out, 0, std::ptr::null())
                };
                if !out.is_null() {
                    subscribed = unsafe { cf::CFRetain(out) as *mut c_void };
                }
                unsafe { CFRelease(channels) };
            }
        }
        Self {
            api,
            subscription,
            subscribed,
            previous: None,
            tables: frequency_tables(),
            thermal: Thermal::new(),
        }
    }

    pub fn sample(&mut self) -> SocReading {
        let mut reading = SocReading {
            temperature_c: self.thermal.die_temperature(),
            ..SocReading::default()
        };
        let Some(api) = &self.api else { return reading };
        if self.subscription.is_null() || self.subscribed.is_null() {
            return reading;
        }
        let current =
            unsafe { (api.samples)(self.subscription, self.subscribed, std::ptr::null()) };
        if current.is_null() {
            return reading;
        }
        let now = Instant::now();
        let Some((previous, then)) = self.previous.take() else {
            self.previous = Some((current, now));
            return reading;
        };
        let elapsed = now.duration_since(then).as_secs_f64();
        if elapsed > 0.05 {
            let delta = unsafe { (api.delta)(previous, current, std::ptr::null()) };
            if !delta.is_null() {
                parse(api, delta, elapsed, &self.tables, &mut reading);
                unsafe { CFRelease(delta) };
            }
        }
        unsafe { CFRelease(previous) };
        self.previous = Some((current, now));
        reading
    }
}

fn cf_string_copy(api: &ReportApi, group: &str, subgroup: Option<&str>) -> Option<*mut c_void> {
    let group = cf::cf_string(group);
    let subgroup_cf = subgroup.map(cf::cf_string).unwrap_or(std::ptr::null());
    let channels = unsafe { (api.copy)(group, subgroup_cf, 0, 0, 0) };
    unsafe {
        if !group.is_null() {
            CFRelease(group);
        }
        if !subgroup_cf.is_null() {
            CFRelease(subgroup_cf);
        }
    }
    if channels.is_null() {
        None
    } else {
        Some(channels)
    }
}

fn parse(
    api: &ReportApi,
    delta: *const c_void,
    seconds: f64,
    tables: &HashMap<String, Vec<f64>>,
    reading: &mut SocReading,
) {
    let channels = cf::dict_get(delta, "IOReportChannels");
    if channels.is_null() {
        return;
    }
    let count = unsafe { cf::CFArrayGetCount(channels) };
    let mut energy: HashMap<String, f64> = HashMap::new();
    let mut cluster_states: HashMap<String, (f64, f64)> = HashMap::new();
    for index in 0..count {
        let channel = unsafe { cf::CFArrayGetValueAtIndex(channels, index) };
        let group = api.text(api.group, channel);
        let name = api.text(api.name, channel);
        if group == "Energy Model" {
            let unit = api.text(api.unit, channel);
            if let Some(joules) = SocMath::joules(unsafe { (api.integer)(channel, 0) }, &unit) {
                *energy.entry(name).or_insert(0.0) += joules;
            }
            continue;
        }
        let state_count = unsafe { (api.state_count)(channel) };
        if !(0..64).contains(&state_count) {
            continue;
        }
        let states: Vec<(String, i64)> = (0..state_count)
            .map(|index| {
                let label =
                    cf::string(unsafe { (api.state_name)(channel, index) }).unwrap_or_default();
                (label, unsafe { (api.state_residency)(channel, index) })
            })
            .collect();
        let active_count = states.iter().filter(|(name, _)| !is_idle(name)).count();
        if group == "CPU Stats" {
            if let Some(kind) = SocMath::cluster_type(&name) {
                let table = cpu_table(tables, &kind, active_count);
                let (hertz, _) = SocMath::residency(&states, &table);
                let active_residency: f64 = states
                    .iter()
                    .filter(|(name, _)| !is_idle(name))
                    .map(|(_, residency)| (*residency).max(0) as f64)
                    .sum();
                if let Some(hertz) = hertz {
                    let entry = cluster_states.entry(kind).or_insert((0.0, 0.0));
                    entry.0 += hertz * active_residency;
                    entry.1 += active_residency;
                }
            }
        } else if group == "GPU Stats" && name == "GPUPH" {
            let table = pick(
                tables,
                &[
                    "voltage-states9",
                    "voltage-states8",
                    "voltage-states14",
                    "voltage-states31",
                ],
                active_count,
            );
            let (hertz, active) = SocMath::residency(&states, &table);
            reading.gpu_hz = hertz;
            reading.gpu_active = Some(active);
        }
    }
    let watts = |joules: f64| joules / seconds;
    if let Some(cpu) = energy.get("CPU Energy") {
        reading.cpu_watts = Some(watts(*cpu));
    }
    if let Some(gpu) = energy.get("GPU Energy").or_else(|| energy.get("GPU")) {
        reading.gpu_watts = Some(watts(*gpu));
    }
    if let Some(ane) = energy.get("ANE") {
        reading.ane_watts = Some(watts(*ane));
    }
    if let Some(dram) = energy.get("DRAM") {
        reading.dram_watts = Some(watts(*dram));
    }
    for (name, joules) in &energy {
        if let Some(kind) = SocMath::cluster_type(name) {
            *reading.cluster_watts.entry(kind).or_insert(0.0) += watts(*joules);
        }
    }
    for (kind, (weighted, active)) in cluster_states {
        if active > 0.0 {
            reading.cluster_hz.insert(kind, weighted / active);
        }
    }
}

fn cpu_table(tables: &HashMap<String, Vec<f64>>, kind: &str, states: usize) -> Vec<f64> {
    let preferred: &[&str] = match kind {
        "E" => &["voltage-states1-sram"],
        "P" | "S" => &["voltage-states5-sram"],
        "M" => &["voltage-states22-sram", "voltage-states23-sram"],
        _ => &[],
    };
    pick(tables, preferred, states)
}

fn pick(tables: &HashMap<String, Vec<f64>>, preferred: &[&str], states: usize) -> Vec<f64> {
    preferred
        .iter()
        .find_map(|key| {
            tables
                .get(*key)
                .filter(|table| table.len() == states)
                .cloned()
        })
        .unwrap_or_default()
}

fn frequency_tables() -> HashMap<String, Vec<f64>> {
    let mut tables = HashMap::new();
    let Ok(path) = CString::new("IODeviceTree:/arm-io/pmgr") else {
        return tables;
    };
    let entry = unsafe { cf::IORegistryEntryFromPath(0, path.as_ptr()) };
    if entry == 0 {
        return tables;
    }
    let properties = cf::entry_properties(entry);
    unsafe { cf::IOObjectRelease(entry) };
    if properties.is_null() {
        return tables;
    }
    // The property names are stable across Apple Silicon generations; unknown chips simply
    // leave frequency blank rather than borrowing another block's table.
    for index in 0..40 {
        for suffix in ["", "-sram"] {
            let key = format!("voltage-states{index}{suffix}");
            let value = cf::dict_get(properties, &key);
            if let Some(bytes) = cf::data_bytes(value) {
                let hertz = SocMath::frequencies(&bytes);
                if !hertz.is_empty() {
                    tables.insert(key, hertz);
                }
            }
        }
    }
    unsafe { CFRelease(properties) };
    tables
}

struct Thermal {
    services: Vec<*const c_void>,
    copy_event: Option<unsafe extern "C" fn(*const c_void, i64, i32, i64) -> *const c_void>,
    float_value: Option<unsafe extern "C" fn(*const c_void, i32) -> f64>,
}

impl Thermal {
    fn new() -> Self {
        let mut thermal = Self {
            services: Vec::new(),
            copy_event: None,
            float_value: None,
        };
        unsafe fn hid<T>(name: &[u8]) -> Option<T> {
            let pointer = libc::dlsym(libc::RTLD_DEFAULT, name.as_ptr().cast());
            if pointer.is_null() {
                None
            } else {
                Some(std::mem::transmute_copy(&pointer))
            }
        }
        let Some(create): Option<unsafe extern "C" fn(*const c_void) -> *mut c_void> =
            (unsafe { hid(b"IOHIDEventSystemClientCreate\0") })
        else {
            return thermal;
        };
        let Some(set_matching): Option<unsafe extern "C" fn(*mut c_void, *const c_void) -> i32> =
            (unsafe { hid(b"IOHIDEventSystemClientSetMatching\0") })
        else {
            return thermal;
        };
        let Some(copy_services): Option<unsafe extern "C" fn(*mut c_void) -> *const c_void> =
            (unsafe { hid(b"IOHIDEventSystemClientCopyServices\0") })
        else {
            return thermal;
        };
        let Some(copy_property): Option<
            unsafe extern "C" fn(*const c_void, *const c_void) -> *const c_void,
        > = (unsafe { hid(b"IOHIDServiceClientCopyProperty\0") }) else {
            return thermal;
        };
        thermal.copy_event = unsafe { hid(b"IOHIDServiceClientCopyEvent\0") };
        thermal.float_value = unsafe { hid(b"IOHIDEventGetFloatValue\0") };
        let client = unsafe { create(std::ptr::null()) };
        if client.is_null() {
            return thermal;
        }
        if let Some(matching) = usage_matching() {
            unsafe { set_matching(client, matching) };
            unsafe { CFRelease(matching) };
        }
        let services = unsafe { copy_services(client) };
        if !services.is_null() {
            let count = unsafe { cf::CFArrayGetCount(services) };
            let product = cf::cf_string("Product");
            for index in 0..count {
                let service = unsafe { cf::CFArrayGetValueAtIndex(services, index) };
                let name = cf::string(unsafe { copy_property(service, product) });
                if name
                    .as_deref()
                    .is_some_and(|name| name.starts_with("PMU tdie"))
                {
                    unsafe { cf::CFRetain(service) };
                    thermal.services.push(service);
                }
            }
            unsafe { CFRelease(product) };
            unsafe { CFRelease(services) };
        }
        thermal
    }

    fn die_temperature(&self) -> Option<f64> {
        let (Some(copy_event), Some(float_value)) = (self.copy_event, self.float_value) else {
            return None;
        };
        let mut values = Vec::new();
        for service in &self.services {
            let event = unsafe { copy_event(*service, 15, 0, 0) };
            if event.is_null() {
                continue;
            }
            let value = unsafe { float_value(event, 15 << 16) };
            unsafe { CFRelease(event) };
            if (0.0..130.0).contains(&value) {
                values.push(value);
            }
        }
        if values.is_empty() {
            None
        } else {
            Some(values.iter().sum::<f64>() / values.len() as f64)
        }
    }
}

fn usage_matching() -> Option<*const c_void> {
    let page_key = cf::cf_string("PrimaryUsagePage");
    let usage_key = cf::cf_string("PrimaryUsage");
    let mut page = 0xff00i32;
    let mut usage = 5i32;
    let page_number =
        unsafe { cf::CFNumberCreate(std::ptr::null(), 9, (&mut page as *mut i32).cast()) };
    let usage_number =
        unsafe { cf::CFNumberCreate(std::ptr::null(), 9, (&mut usage as *mut i32).cast()) };
    if page_key.is_null() || usage_key.is_null() || page_number.is_null() || usage_number.is_null()
    {
        return None;
    }
    let keys = [page_key, usage_key];
    let values = [page_number, usage_number];
    let dict = unsafe {
        cf::CFDictionaryCreate(
            std::ptr::null(),
            keys.as_ptr(),
            values.as_ptr(),
            2,
            std::ptr::addr_of!(cf::kCFTypeDictionaryKeyCallBacks).cast(),
            std::ptr::addr_of!(cf::kCFTypeDictionaryValueCallBacks).cast(),
        )
    };
    for value in [page_key, usage_key, page_number, usage_number] {
        unsafe { CFRelease(value) };
    }
    if dict.is_null() {
        None
    } else {
        Some(dict)
    }
}

#[cfg(test)]
mod tests {
    use super::SocMath;

    #[test]
    fn energy_units() {
        assert!((SocMath::joules(1_500, "mJ").unwrap() - 1.5).abs() < 1e-9);
        assert!((SocMath::joules(2_000_000, "uJ").unwrap() - 2.0).abs() < 1e-9);
        assert!((SocMath::joules(956_000_000, "nJ").unwrap() - 0.956).abs() < 1e-9);
        assert!(SocMath::joules(1, "W").is_none());
    }

    #[test]
    fn frequency_tables() {
        let kilohertz = SocMath::frequencies(&table(&[(1_308_000, 600), (4_608_000, 900)]));
        assert_eq!(kilohertz, vec![1_308_000_000.0, 4_608_000_000.0]);
        let hertz = SocMath::frequencies(&table(&[
            (0, 125),
            (338_000_000, 600),
            (1_620_000_000, 660),
        ]));
        assert_eq!(hertz, vec![338_000_000.0, 1_620_000_000.0]);
        assert!(SocMath::frequencies(&table(&[(50_103, 620), (14_222, 900)])).is_empty());
    }

    #[test]
    fn residency_weights_active_states() {
        let states = vec![
            ("IDLE".into(), 50),
            ("V0P1".into(), 25),
            ("V1P0".into(), 25),
        ];
        let (hertz, active) = SocMath::residency(&states, &[1e9, 3e9]);
        assert!((hertz.unwrap() - 2e9).abs() < 1.0);
        assert!((active - 0.5).abs() < 1e-9);
        let (idle_hz, idle) =
            SocMath::residency(&[("OFF".into(), 100), ("P1".into(), 0)], &[338e6]);
        assert!(idle_hz.is_none());
        assert_eq!(idle, 0.0);
    }

    #[test]
    fn cluster_type_from_channel() {
        assert_eq!(SocMath::cluster_type("PCPU").as_deref(), Some("P"));
        assert_eq!(SocMath::cluster_type("MCPU1").as_deref(), Some("M"));
        assert_eq!(SocMath::cluster_type("ECPU0").as_deref(), Some("E"));
        assert!(SocMath::cluster_type("PCPM").is_none());
        assert!(SocMath::cluster_type("MCPU0_0").is_none());
        assert!(SocMath::cluster_type("CPU Energy").is_none());
    }

    fn table(pairs: &[(u32, u32)]) -> Vec<u8> {
        let mut data = Vec::new();
        for (frequency, voltage) in pairs {
            data.extend_from_slice(&frequency.to_le_bytes());
            data.extend_from_slice(&voltage.to_le_bytes());
        }
        data
    }
}
