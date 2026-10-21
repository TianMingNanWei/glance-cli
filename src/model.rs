use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostInfo {
    pub host_name: String,
    pub chip: String,
    pub model: String,
    pub os_version: String,
    pub os_build: String,
    pub logical_cores: u32,
    pub memory_bytes: u64,
    pub uptime_seconds: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cluster {
    pub name: String,
    pub kind: String,
    pub core_count: u32,
    pub utilization: f64,
    pub frequency_hz: Option<f64>,
    pub power_watts: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Core {
    pub id: u32,
    pub level: u32,
    pub utilization: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cpu {
    pub utilization: f64,
    pub power_watts: Option<f64>,
    pub temperature_c: Option<f64>,
    pub clusters: Vec<Cluster>,
    pub cores: Vec<Core>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Gpu {
    pub name: String,
    pub utilization: Option<f64>,
    pub memory_bytes: Option<u64>,
    pub frequency_hz: Option<f64>,
    pub power_watts: Option<f64>,
    pub active: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Npu {
    pub supported: bool,
    pub power_watts: Option<f64>,
    pub active: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Memory {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub wired_bytes: u64,
    pub active_bytes: u64,
    pub inactive_bytes: u64,
    pub compressed_bytes: u64,
    pub free_bytes: u64,
    pub swap_bytes: u64,
    pub pressure: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub read_bytes_per_second: Option<f64>,
    pub write_bytes_per_second: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Battery {
    pub fraction: f64,
    pub health: Option<f64>,
    pub cycles: Option<u32>,
    pub charging: bool,
    pub external_power: bool,
    pub minutes_remaining: Option<u32>,
    /// Positive while charging, negative while discharging.
    pub battery_watts: Option<f64>,
    pub charger: Option<Charger>,
}

/// The connected power adapter: what it is rated for and what the Mac draws from it now.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Charger {
    pub rated_watts: Option<f64>,
    pub rated_volts: Option<f64>,
    pub rated_amps: Option<f64>,
    pub input_watts: Option<f64>,
    pub input_volts: Option<f64>,
    pub input_amps: Option<f64>,
    pub wireless: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Network {
    pub download_bytes_per_second: f64,
    pub upload_bytes_per_second: f64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Power {
    pub cpu_watts: Option<f64>,
    pub gpu_watts: Option<f64>,
    pub npu_watts: Option<f64>,
    pub memory_watts: Option<f64>,
    pub chip_watts: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Process {
    pub pid: i32,
    pub name: String,
    pub identifier: String,
    pub threads: u32,
    pub cpu: f64,
    pub memory_bytes: u64,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub host: HostInfo,
    pub cpu: Cpu,
    pub gpu: Gpu,
    pub npu: Npu,
    pub memory: Memory,
    pub disk: Disk,
    pub battery: Option<Battery>,
    pub network: Network,
    pub power: Power,
    pub processes: Vec<Process>,
}
