use crate::format::{
    bytes, bytes_per_second, celsius, duration, hertz, meter, percent, sparkline, watts,
};
use crate::model::{Cluster, Snapshot};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

#[derive(Default)]
pub struct History {
    pub cpu: Vec<f64>,
    pub gpu: Vec<f64>,
    pub download: Vec<f64>,
    pub upload: Vec<f64>,
}

impl History {
    pub fn push(&mut self, snapshot: &Snapshot) {
        push(&mut self.cpu, snapshot.cpu.utilization);
        push(&mut self.gpu, snapshot.gpu.utilization.unwrap_or(0.0));
        push(
            &mut self.download,
            snapshot.network.download_bytes_per_second,
        );
        push(&mut self.upload, snapshot.network.upload_bytes_per_second);
    }
}

fn push(values: &mut Vec<f64>, value: f64) {
    values.push(value);
    if values.len() > 80 {
        values.drain(..values.len() - 80);
    }
}

pub fn draw(frame: &mut Frame, snapshot: &Snapshot, history: &History) {
    let area = frame.area();
    if area.width < 80 || area.height < 24 {
        frame.render_widget(
            Paragraph::new("Resize the terminal to at least 80×24."),
            area,
        );
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Percentage(46),
            Constraint::Percentage(28),
            Constraint::Min(6),
            Constraint::Length(1),
        ])
        .split(area);
    frame.render_widget(header(snapshot), rows[0]);
    let top = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[1]);
    frame.render_widget(cpu_panel(snapshot, history, top[0]), top[0]);
    frame.render_widget(accelerator_panel(snapshot, history, top[1]), top[1]);
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(40),
            Constraint::Percentage(30),
            Constraint::Percentage(30),
        ])
        .split(rows[2]);
    frame.render_widget(memory_panel(snapshot, middle[0]), middle[0]);
    frame.render_widget(network_panel(snapshot, history, middle[1]), middle[1]);
    frame.render_widget(disk_panel(snapshot, middle[2]), middle[2]);
    frame.render_widget(process_panel(snapshot, rows[3]), rows[3]);
    frame.render_widget(
        Paragraph::new(" q quit    c sort by CPU    m sort by memory    every core, sensor, and process is on this screen "),
        rows[4],
    );
}

fn header(snapshot: &Snapshot) -> Paragraph<'static> {
    let host = &snapshot.host;
    let chip = if host.chip.is_empty() {
        host.model.clone()
    } else {
        format!("{}  {}", host.chip, host.model)
    };
    Paragraph::new(Line::from(vec![
        Span::styled(
            " Glance ",
            Style::new()
                .fg(Color::Black)
                .bg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(
            "  {}   {chip}   macOS {} ({})   up {}",
            host.host_name,
            host.os_version,
            host.os_build,
            duration(host.uptime_seconds)
        )),
    ]))
}

fn panel(title: String, lines: Vec<Line<'static>>, color: Color) -> Paragraph<'static> {
    Paragraph::new(lines).block(
        Block::default()
            .title(format!(" {title} "))
            .title_style(Style::new().fg(color).add_modifier(Modifier::BOLD))
            .borders(Borders::ALL)
            .border_style(Style::new().fg(color)),
    )
}

fn cpu_panel(snapshot: &Snapshot, history: &History, area: Rect) -> Paragraph<'static> {
    let width = area.width.saturating_sub(4) as usize;
    let mut lines = vec![
        Line::from(sparkline(&history.cpu, width, Some(1.0))),
        Line::from(vec![
            Span::raw("load "),
            Span::styled(
                percent(snapshot.cpu.utilization),
                Style::new().fg(heat(snapshot.cpu.utilization)),
            ),
            Span::raw(format!(
                "    {}",
                snapshot
                    .cpu
                    .power_watts
                    .map(watts)
                    .unwrap_or_else(|| "n/a".into())
            )),
            Span::raw(format!(
                "    {}",
                snapshot
                    .cpu
                    .temperature_c
                    .map(celsius)
                    .unwrap_or_else(|| "n/a".into())
            )),
        ]),
    ];
    for cluster in &snapshot.cpu.clusters {
        lines.push(cluster_line(cluster));
    }
    let room = area.height.saturating_sub(2) as usize;
    let used = lines.len();
    let core_rows = room.saturating_sub(used);
    let columns = if snapshot.cpu.cores.len() > core_rows.max(1) {
        2
    } else {
        1
    };
    let per_column = snapshot.cpu.cores.len().div_ceil(columns);
    for row in 0..core_rows.min(per_column) {
        let mut spans = Vec::new();
        for column in 0..columns {
            let index = column * per_column + row;
            if let Some(core) = snapshot.cpu.cores.get(index) {
                let bar_width = (width / columns).saturating_sub(12).clamp(4, 24);
                spans.push(Span::raw(format!("C{:<2} ", core.id)));
                spans.push(Span::styled(
                    meter(core.utilization, bar_width),
                    Style::new().fg(heat(core.utilization)),
                ));
                spans.push(Span::styled(
                    format!(" {:>6}  ", percent(core.utilization)),
                    Style::new().fg(heat(core.utilization)),
                ));
            }
        }
        lines.push(Line::from(spans));
    }
    let title = format!("CPU  {}", percent(snapshot.cpu.utilization));
    panel(title, lines, Color::Cyan)
}

fn cluster_line(cluster: &Cluster) -> Line<'static> {
    let name = if cluster.name.is_empty() {
        cluster.kind.clone()
    } else {
        cluster.name.clone()
    };
    Line::from(vec![
        Span::styled(
            format!("{name} ×{}  ", cluster.core_count),
            Style::new().fg(Color::White),
        ),
        Span::raw(format!(
            "{:<10}",
            cluster
                .frequency_hz
                .map(hertz)
                .unwrap_or_else(|| "n/a".into())
        )),
        Span::raw(format!(
            "{:<10}",
            cluster
                .power_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into())
        )),
        Span::styled(
            percent(cluster.utilization),
            Style::new().fg(heat(cluster.utilization)),
        ),
    ])
}

fn accelerator_panel(snapshot: &Snapshot, history: &History, area: Rect) -> Paragraph<'static> {
    let width = area.width.saturating_sub(4) as usize;
    let gpu = &snapshot.gpu;
    let npu_state = match snapshot.npu.active {
        Some(true) => "active",
        Some(false) => "idle",
        None => "n/a",
    };
    let lines = vec![
        Line::from(Span::styled(
            format!(
                "GPU  {}",
                gpu.utilization.map(percent).unwrap_or_else(|| "n/a".into())
            ),
            Style::new().fg(Color::Magenta).add_modifier(Modifier::BOLD),
        )),
        Line::from(sparkline(&history.gpu, width, Some(1.0))),
        labeled(
            "memory",
            &gpu.memory_bytes.map(bytes).unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "frequency",
            &gpu.frequency_hz.map(hertz).unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "power",
            &gpu.power_watts.map(watts).unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "active",
            &gpu.active.map(percent).unwrap_or_else(|| "n/a".into()),
        ),
        Line::from(""),
        Line::from(Span::styled(
            format!("Neural Engine  {npu_state}"),
            Style::new().fg(Color::Green).add_modifier(Modifier::BOLD),
        )),
        labeled(
            "power",
            &snapshot
                .npu
                .power_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        Line::from(""),
        Line::from(Span::styled(
            "Chip power",
            Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        )),
        labeled(
            "CPU",
            &snapshot
                .power
                .cpu_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "GPU",
            &snapshot
                .power
                .gpu_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "NPU",
            &snapshot
                .power
                .npu_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "memory",
            &snapshot
                .power
                .memory_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "total",
            &snapshot
                .power
                .chip_watts
                .map(watts)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "die",
            &snapshot
                .cpu
                .temperature_c
                .map(celsius)
                .unwrap_or_else(|| "n/a".into()),
        ),
    ];
    let _ = area;
    panel("GPU / NPU".into(), lines, Color::Magenta)
}

fn memory_panel(snapshot: &Snapshot, area: Rect) -> Paragraph<'static> {
    let memory = &snapshot.memory;
    let total = memory.total_bytes.max(1) as f64;
    let used = memory.used_bytes as f64 / total;
    let width = area.width.saturating_sub(16) as usize;
    let row = |label: &str, bytes_value: u64| {
        let fraction = bytes_value as f64 / total;
        Line::from(format!(
            "{label:<10} {:>9}  {}",
            bytes(bytes_value),
            meter(fraction, width.min(18))
        ))
    };
    let lines = vec![
        Line::from(format!(
            "{} / {}",
            bytes(memory.used_bytes),
            bytes(memory.total_bytes)
        )),
        Line::from(meter(used, area.width.saturating_sub(4) as usize)),
        row("wired", memory.wired_bytes),
        row("active", memory.active_bytes),
        row("compressed", memory.compressed_bytes),
        row("inactive", memory.inactive_bytes),
        row("free", memory.free_bytes),
        row("swap", memory.swap_bytes),
        Line::from(format!("pressure   {}", memory.pressure)),
    ];
    panel(format!("Memory  {}", percent(used)), lines, Color::Yellow)
}

fn network_panel(snapshot: &Snapshot, history: &History, area: Rect) -> Paragraph<'static> {
    let width = area.width.saturating_sub(4) as usize;
    let network = &snapshot.network;
    let lines = vec![
        Line::from(Span::styled(
            format!("↓ {}", bytes_per_second(network.download_bytes_per_second)),
            Style::new().fg(Color::Cyan),
        )),
        Line::from(sparkline(&history.download, width, None)),
        Line::from(Span::styled(
            format!("↑ {}", bytes_per_second(network.upload_bytes_per_second)),
            Style::new().fg(Color::LightMagenta),
        )),
        Line::from(sparkline(&history.upload, width, None)),
    ];
    panel("Network".into(), lines, Color::Cyan)
}

fn disk_panel(snapshot: &Snapshot, area: Rect) -> Paragraph<'static> {
    let disk = &snapshot.disk;
    let used = disk
        .total_bytes
        .saturating_sub(disk.free_bytes.min(disk.total_bytes));
    let fraction = if disk.total_bytes > 0 {
        used as f64 / disk.total_bytes as f64
    } else {
        0.0
    };
    let mut lines = vec![
        Line::from(format!("{} / {}", bytes(used), bytes(disk.total_bytes))),
        Line::from(meter(fraction, area.width.saturating_sub(4) as usize)),
        labeled("free", &bytes(disk.free_bytes)),
        labeled(
            "read",
            &disk
                .read_bytes_per_second
                .map(bytes_per_second)
                .unwrap_or_else(|| "n/a".into()),
        ),
        labeled(
            "write",
            &disk
                .write_bytes_per_second
                .map(bytes_per_second)
                .unwrap_or_else(|| "n/a".into()),
        ),
    ];
    if let Some(battery) = &snapshot.battery {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            format!("Battery  {}", percent(battery.fraction)),
            Style::new().fg(Color::Green),
        )));
        lines.push(Line::from(meter(
            battery.fraction,
            area.width.saturating_sub(4) as usize,
        )));
        lines.push(labeled(
            "health",
            &battery.health.map(percent).unwrap_or_else(|| "n/a".into()),
        ));
        lines.push(labeled(
            "cycles",
            &battery
                .cycles
                .map(|count| count.to_string())
                .unwrap_or_else(|| "n/a".into()),
        ));
    }
    panel(format!("Disk  {}", percent(fraction)), lines, Color::Blue)
}

fn process_panel(snapshot: &Snapshot, area: Rect) -> Paragraph<'static> {
    let rows = area.height.saturating_sub(3) as usize;
    let mut lines = vec![Line::from(Span::styled(
        format!(
            "{:<7}{:<18}{:<8}{:>10}{:>8}  {}",
            "PID", "Name", "Threads", "Memory", "CPU", "Bundle ID or path"
        ),
        Style::new().fg(Color::DarkGray),
    ))];
    for process in snapshot.processes.iter().take(rows) {
        lines.push(Line::from(vec![
            Span::raw(format!(
                "{:<7}{:<18.18}{:<8}{:>10} ",
                process.pid,
                process.name,
                process.threads,
                bytes(process.memory_bytes)
            )),
            Span::styled(
                format!("{:>7}  ", percent(process.cpu)),
                Style::new().fg(heat(process.cpu.min(1.0))),
            ),
            Span::styled(
                shrink_middle(&process.identifier, 48),
                Style::new().fg(Color::DarkGray),
            ),
        ]));
    }
    panel(
        format!("Processes  {}", snapshot.processes.len()),
        lines,
        Color::LightMagenta,
    )
}

fn labeled(name: &str, value: &str) -> Line<'static> {
    Line::from(format!("{name:<10} {value}"))
}

fn shrink_middle(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width < 5 {
        return chars.into_iter().take(width).collect();
    }
    let head = (width - 1) / 2;
    let tail = width - head - 1;
    format!(
        "{}…{}",
        chars.iter().take(head).collect::<String>(),
        chars.iter().skip(chars.len() - tail).collect::<String>()
    )
}

fn heat(fraction: f64) -> Color {
    match fraction {
        value if value < 0.35 => Color::Green,
        value if value < 0.65 => Color::Yellow,
        value if value < 0.85 => Color::Rgb(255, 160, 60),
        _ => Color::Red,
    }
}
