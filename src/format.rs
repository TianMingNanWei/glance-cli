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

/// Block chart. `scale` is the value that fills the row; `None` uses the series peak.
pub fn sparkline(values: &[f64], width: usize, scale: Option<f64>) -> String {
    const TICKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    if width == 0 || values.is_empty() {
        return String::new();
    }
    let peak = scale.unwrap_or_else(|| values.iter().copied().fold(0.0, f64::max).max(1e-9));
    let start = values.len().saturating_sub(width);
    values[start..]
        .iter()
        .map(|value| {
            let index = ((value / peak).clamp(0.0, 1.0) * 8.0).round() as usize;
            TICKS[index.min(8)]
        })
        .collect()
}

pub fn meter(fraction: f64, width: usize) -> String {
    let width = width.max(1);
    let filled = (fraction.clamp(0.0, 1.0) * width as f64).round() as usize;
    format!("{}{}", "█".repeat(filled), "░".repeat(width - filled))
}
