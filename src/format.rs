pub fn percent(fraction: f64) -> String {
    format!("{:.1}%", fraction * 100.0)
}

pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut amount = value as f64;
    let mut unit = 0;
    while amount >= 1000.0 && unit < UNITS.len() - 1 {
        amount /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{value} B")
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

pub fn bytes_per_second(value: f64) -> String {
    format!("{}/s", bytes(value.max(0.0) as u64))
}

pub fn watts(value: f64) -> String {
    format!("{value:.2} W")
}

pub fn hertz(value: f64) -> String {
    if value >= 1_000_000_000.0 {
        format!("{:.2} GHz", value / 1_000_000_000.0)
    } else {
        format!("{:.0} MHz", value / 1_000_000.0)
    }
}

pub fn celsius(value: f64) -> String {
    format!("{value:.1}°C")
}

pub fn duration(seconds: u64) -> String {
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}
