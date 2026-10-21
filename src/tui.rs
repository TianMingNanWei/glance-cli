use crate::about;
use crate::anim::Animator;
use crate::format::{bytes, bytes_per_second, celsius, duration, hertz, percent, watts};
use crate::model::Process;
use crate::model::{Charger, Cluster, Snapshot};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::Marker;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Axis, Bar, BarChart, BarGroup, Block, BorderType, Borders, Cell, Chart, Clear, Dataset,
    GraphType, Padding, Paragraph, Row, Sparkline, SparklineBar, Table, TableState,
};
use ratatui::Frame;
use std::cmp::Ordering;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

const HISTORY: usize = 120;

const CPU: Color = Color::Rgb(90, 200, 255);
const GPU: Color = Color::Rgb(190, 120, 255);
const NPU: Color = Color::Rgb(80, 230, 170);
const POWER: Color = Color::Rgb(255, 200, 70);
const MEMORY: Color = Color::Rgb(255, 160, 70);
const NETWORK: Color = Color::Rgb(70, 220, 230);
const UPLOAD: Color = Color::Rgb(255, 110, 200);
const DISK: Color = Color::Rgb(120, 130, 255);
const BATTERY: Color = Color::Rgb(110, 230, 120);
const PROCESS: Color = Color::Rgb(255, 120, 170);
const DIM: Color = Color::Rgb(110, 115, 135);
const FAINT: Color = Color::Rgb(55, 58, 72);
const TEXT: Color = Color::Rgb(225, 228, 240);
const ALERT: Color = Color::Rgb(255, 70, 95);
const TRACK: Color = Color::Rgb(30, 32, 42);

#[derive(Clone, Copy, PartialEq, Eq)]
enum SortKey {
    Cpu,
    Memory,
    Pid,
    Name,
    Threads,
}

impl SortKey {
    fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "CPU",
            SortKey::Memory => "memory",
            SortKey::Pid => "PID",
            SortKey::Name => "name",
            SortKey::Threads => "threads",
        }
    }

    /// Numbers read best largest first, names alphabetically.
    fn descending_by_default(self) -> bool {
        matches!(self, SortKey::Cpu | SortKey::Memory | SortKey::Threads)
    }
}

enum Mode {
    Normal,
    Filter,
    Help,
    About,
    Confirm { pid: i32, name: String, force: bool },
}

struct Toast {
    text: String,
    color: Color,
    shown: Instant,
}

pub struct App {
    pub history: History,
    pub anim: Animator,
    pub paused: bool,
    pub interval: Duration,
    /// How far the wait for the next sample has come, from 0 to 1.
    pub refresh_progress: f64,
    table: TableState,
    selected_pid: Option<i32>,
    sort: SortKey,
    reversed: bool,
    filter: String,
    mode: Mode,
    toast: Option<Toast>,
    process_area: Rect,
}

impl App {
    pub fn new() -> Self {
        Self {
            history: History::default(),
            anim: Animator::new(),
            paused: false,
            interval: Duration::from_secs(1),
            refresh_progress: 0.0,
            table: TableState::default().with_selected(Some(0)),
            selected_pid: None,
            sort: SortKey::Cpu,
            reversed: false,
            filter: String::new(),
            mode: Mode::Normal,
            toast: None,
            process_area: Rect::default(),
        }
    }

    /// Frames per second the live loop should aim for.
    pub fn frame_interval(&self) -> Duration {
        if self.anim.enabled {
            Duration::from_millis(33)
        } else {
            Duration::from_millis(200)
        }
    }

    /// Processes after the search filter and the chosen sort.
    fn visible<'a>(&self, snapshot: &'a Snapshot) -> Vec<&'a Process> {
        let needle = self.filter.to_lowercase();
        let mut list: Vec<&Process> = snapshot
            .processes
            .iter()
            .filter(|process| {
                needle.is_empty()
                    || process.name.to_lowercase().contains(&needle)
                    || process.identifier.to_lowercase().contains(&needle)
                    || process.pid.to_string().starts_with(&needle)
            })
            .collect();
        list.sort_by(|left, right| {
            let order = match self.sort {
                SortKey::Cpu => left.cpu.partial_cmp(&right.cpu).unwrap_or(Ordering::Equal),
                SortKey::Memory => left.memory_bytes.cmp(&right.memory_bytes),
                SortKey::Pid => left.pid.cmp(&right.pid),
                SortKey::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
                SortKey::Threads => left.threads.cmp(&right.threads),
            };
            if self.sort.descending_by_default() != self.reversed {
                order.reverse()
            } else {
                order
            }
        });
        list
    }

    fn select(&mut self, delta: i64, visible: &[&Process]) {
        if visible.is_empty() {
            self.table.select(None);
            self.selected_pid = None;
            return;
        }
        let current = self.table.selected().unwrap_or(0) as i64;
        let next = (current + delta).clamp(0, visible.len() as i64 - 1) as usize;
        self.table.select(Some(next));
        self.selected_pid = Some(visible[next].pid);
    }

    fn set_sort(&mut self, key: SortKey) {
        if self.sort == key {
            self.reversed = !self.reversed;
        } else {
            self.sort = key;
            self.reversed = false;
        }
        self.notify(format!("Sorted by {}", self.sort.label()), PROCESS);
    }

    fn faster(&mut self) {
        self.interval = Duration::from_millis(
            (self.interval.as_millis() as u64)
                .saturating_sub(250)
                .max(250),
        );
        self.notify(
            format!("Refresh every {} ms", self.interval.as_millis()),
            CPU,
        );
    }

    fn slower(&mut self) {
        self.interval = Duration::from_millis((self.interval.as_millis() as u64 + 250).min(5_000));
        self.notify(
            format!("Refresh every {} ms", self.interval.as_millis()),
            CPU,
        );
    }

    fn notify(&mut self, text: String, color: Color) {
        self.toast = Some(Toast {
            text,
            color,
            shown: Instant::now(),
        });
    }

    /// Handles one key press. Returns true when the user asked to quit.
    pub fn handle_key(&mut self, key: KeyEvent, snapshot: &Snapshot) -> bool {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return true;
        }
        let visible = self.visible(snapshot);
        match &self.mode {
            Mode::Help | Mode::About => {
                if key.code == KeyCode::Char('q') {
                    return true;
                }
                self.mode = Mode::Normal;
                return false;
            }
            Mode::Confirm { pid, name, force } => {
                let (pid, name, force) = (*pid, name.clone(), *force);
                self.mode = Mode::Normal;
                if matches!(
                    key.code,
                    KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter
                ) {
                    self.terminate(pid, &name, force);
                } else {
                    self.notify("Cancelled".into(), DIM);
                }
                return false;
            }
            Mode::Filter => {
                match key.code {
                    KeyCode::Enter => self.mode = Mode::Normal,
                    KeyCode::Esc => {
                        self.filter.clear();
                        self.mode = Mode::Normal;
                    }
                    KeyCode::Backspace => {
                        self.filter.pop();
                    }
                    KeyCode::Up => self.select(-1, &visible),
                    KeyCode::Down => self.select(1, &visible),
                    KeyCode::Char(character) => {
                        self.filter.push(character);
                        self.table.select(Some(0));
                        self.selected_pid = None;
                    }
                    _ => {}
                }
                return false;
            }
            Mode::Normal => {}
        }
        let count = visible.len() as i64;
        match key.code {
            KeyCode::Char('q') => return true,
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.notify("Search cleared".into(), DIM);
            }
            KeyCode::Esc => return true,
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => self.mode = Mode::Help,
            KeyCode::Char('i') => self.mode = Mode::About,
            KeyCode::Char('/') => self.mode = Mode::Filter,
            KeyCode::Char('c') => self.set_sort(SortKey::Cpu),
            KeyCode::Char('m') => self.set_sort(SortKey::Memory),
            KeyCode::Char('p') => self.set_sort(SortKey::Pid),
            KeyCode::Char('n') => self.set_sort(SortKey::Name),
            KeyCode::Char('t') => self.set_sort(SortKey::Threads),
            KeyCode::Char('r') => {
                self.reversed = !self.reversed;
                self.notify("Order reversed".into(), PROCESS);
            }
            KeyCode::Up | KeyCode::Char('k') => self.select(-1, &visible),
            KeyCode::Down | KeyCode::Char('j') => self.select(1, &visible),
            KeyCode::PageUp => self.select(-10, &visible),
            KeyCode::PageDown => self.select(10, &visible),
            KeyCode::Home | KeyCode::Char('g') => self.select(-count, &visible),
            KeyCode::End | KeyCode::Char('G') => self.select(count, &visible),
            KeyCode::Char('+') | KeyCode::Char('=') => self.faster(),
            KeyCode::Char('-') => self.slower(),
            KeyCode::Char(' ') => {
                self.paused = !self.paused;
                let text = if self.paused { "Paused" } else { "Resumed" };
                self.notify(text.into(), POWER);
            }
            KeyCode::Char('a') => {
                self.anim.enabled = !self.anim.enabled;
                let text = if self.anim.enabled {
                    "Animations on"
                } else {
                    "Animations off"
                };
                self.notify(text.into(), NPU);
            }
            KeyCode::Char('x') | KeyCode::Char('X') | KeyCode::Delete => {
                let force = key.code == KeyCode::Char('X');
                let selected = self
                    .table
                    .selected()
                    .and_then(|index| visible.get(index))
                    .map(|process| (process.pid, process.name.clone()));
                if let Some((pid, name)) = selected {
                    if pid as u32 == std::process::id() {
                        self.notify("That is Glance itself. Press q to quit.".into(), POWER);
                    } else {
                        self.mode = Mode::Confirm { pid, name, force };
                    }
                }
            }
            _ => {}
        }
        false
    }

    pub fn handle_mouse(&mut self, mouse: MouseEvent, snapshot: &Snapshot) {
        let visible = self.visible(snapshot);
        match mouse.kind {
            MouseEventKind::ScrollDown => self.select(3, &visible),
            MouseEventKind::ScrollUp => self.select(-3, &visible),
            MouseEventKind::Down(MouseButton::Left) => {
                let area = self.process_area;
                let first_row = area.y + 2;
                let inside = mouse.column > area.x
                    && mouse.column < area.right().saturating_sub(1)
                    && mouse.row >= first_row
                    && mouse.row < area.bottom().saturating_sub(1);
                if inside {
                    let index = self.table.offset() + (mouse.row - first_row) as usize;
                    if let Some(process) = visible.get(index) {
                        self.table.select(Some(index));
                        self.selected_pid = Some(process.pid);
                    }
                }
            }
            _ => {}
        }
    }

    fn terminate(&mut self, pid: i32, name: &str, force: bool) {
        let (signal, label) = if force {
            (libc::SIGKILL, "SIGKILL")
        } else {
            (libc::SIGTERM, "SIGTERM")
        };
        if unsafe { libc::kill(pid, signal) } == 0 {
            self.notify(format!("Sent {label} to {name} ({pid})"), BATTERY);
        } else {
            let error = std::io::Error::last_os_error();
            self.notify(format!("Couldn't stop {name}: {error}"), ALERT);
        }
    }
}

#[derive(Default)]
pub struct History {
    cpu: VecDeque<f64>,
    gpu: VecDeque<f64>,
    memory: VecDeque<f64>,
    download: VecDeque<f64>,
    upload: VecDeque<f64>,
    read: VecDeque<f64>,
    write: VecDeque<f64>,
    chip: VecDeque<f64>,
    temperature: VecDeque<f64>,
    cores: HashMap<u32, VecDeque<f64>>,
    peak_download: f64,
    peak_upload: f64,
    total_download: f64,
    total_upload: f64,
}

impl History {
    pub fn push(&mut self, snapshot: &Snapshot, seconds: f64) {
        push(&mut self.cpu, snapshot.cpu.utilization);
        push(&mut self.gpu, snapshot.gpu.utilization.unwrap_or(0.0));
        let total = snapshot.memory.total_bytes.max(1) as f64;
        push(&mut self.memory, snapshot.memory.used_bytes as f64 / total);
        let network = &snapshot.network;
        push(&mut self.download, network.download_bytes_per_second);
        push(&mut self.upload, network.upload_bytes_per_second);
        self.peak_download = self.peak_download.max(network.download_bytes_per_second);
        self.peak_upload = self.peak_upload.max(network.upload_bytes_per_second);
        self.total_download += network.download_bytes_per_second * seconds;
        self.total_upload += network.upload_bytes_per_second * seconds;
        push(
            &mut self.read,
            snapshot.disk.read_bytes_per_second.unwrap_or(0.0),
        );
        push(
            &mut self.write,
            snapshot.disk.write_bytes_per_second.unwrap_or(0.0),
        );
        push(&mut self.chip, snapshot.power.chip_watts.unwrap_or(0.0));
        push(
            &mut self.temperature,
            snapshot.cpu.temperature_c.unwrap_or(0.0),
        );
        for core in &snapshot.cpu.cores {
            push(self.cores.entry(core.id).or_default(), core.utilization);
        }
    }
}

fn push(values: &mut VecDeque<f64>, value: f64) {
    values.push_back(value);
    while values.len() > HISTORY {
        values.pop_front();
    }
}

// MARK: - Layout

pub fn draw(frame: &mut Frame, app: &mut App, snapshot: &Snapshot) {
    let area = frame.area();
    if area.width < 100 || area.height < 30 {
        let message = Paragraph::new(vec![
            Line::from(Span::styled(
                "Glance",
                Style::new().fg(CPU).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "Make the terminal at least 100×30. It is {}×{} now.",
                area.width, area.height
            )),
        ])
        .alignment(Alignment::Center);
        frame.render_widget(message, centered(area, 3));
        return;
    }
    let middle_height = if area.height >= 44 { 12 } else { 10 };
    let flexible = area.height.saturating_sub(4 + middle_height);
    let top_height = (flexible * 55 / 100)
        .max(14)
        .min(flexible.saturating_sub(6));
    let rows = Layout::vertical([
        Constraint::Length(3),
        Constraint::Length(top_height),
        Constraint::Length(middle_height),
        Constraint::Min(6),
        Constraint::Length(1),
    ])
    .split(area);
    header(frame, rows[0], app, snapshot);

    let top =
        Layout::horizontal([Constraint::Percentage(60), Constraint::Percentage(40)]).split(rows[1]);
    cpu_panel(frame, top[0], app, snapshot);
    let side =
        Layout::vertical([Constraint::Percentage(52), Constraint::Percentage(48)]).split(top[1]);
    gpu_panel(frame, side[0], app, snapshot);
    power_panel(frame, side[1], app, snapshot);

    let middle = Layout::horizontal([
        Constraint::Percentage(36),
        Constraint::Percentage(34),
        Constraint::Percentage(30),
    ])
    .split(rows[2]);
    memory_panel(frame, middle[0], app, snapshot);
    network_panel(frame, middle[1], app, snapshot);
    disk_panel(frame, middle[2], app, snapshot);

    process_panel(frame, rows[3], app, snapshot);
    footer(frame, rows[4], app);

    match &app.mode {
        Mode::Help => help_popup(frame, area),
        Mode::About => about_popup(frame, area, app.anim.now()),
        Mode::Confirm { pid, name, force } => confirm_popup(frame, area, *pid, name, *force),
        _ => {}
    }
}

fn centered(area: Rect, height: u16) -> Rect {
    let top = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x: area.x,
        y: top,
        width: area.width,
        height: height.min(area.height),
    }
}

fn panel<'a>(
    width: u16,
    title: &str,
    icon: &str,
    color: Color,
    right: Option<Line<'a>>,
) -> Block<'a> {
    let title_width = title.chars().count() + icon.chars().count() + 7;
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(dim(color, 0.55)))
        .padding(Padding::horizontal(1))
        .title_top(Line::from(vec![
            Span::styled("┤", Style::new().fg(dim(color, 0.55))),
            Span::styled(
                format!(" {icon} "),
                Style::new()
                    .fg(Color::Black)
                    .bg(color)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {title} "),
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ),
            Span::styled("├", Style::new().fg(dim(color, 0.55))),
        ]));
    if let Some(right) =
        right.filter(|right| title_width + line_width(&right.spans) + 3 <= width as usize)
    {
        block = block.title_top(right.right_aligned());
    }
    block
}

fn badge(text: String, color: Color) -> Line<'static> {
    Line::from(vec![
        Span::styled("┤", Style::new().fg(dim(color, 0.55))),
        Span::styled(
            format!(" {text} "),
            Style::new().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::styled("├", Style::new().fg(dim(color, 0.55))),
    ])
}

// MARK: - Header and footer

fn header(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let host = &snapshot.host;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(FAINT));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let fields = vec![
        vec![Span::styled(
            host.host_name.clone(),
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        )],
        vec![Span::styled(host.chip.clone(), Style::new().fg(CPU))],
        vec![
            Span::styled("up ", Style::new().fg(DIM)),
            Span::styled(duration(host.uptime_seconds), Style::new().fg(BATTERY)),
        ],
        vec![Span::styled(
            format!("macOS {} ({})", host.os_version, host.os_build),
            Style::new().fg(TEXT),
        )],
        vec![Span::styled(host.model.clone(), Style::new().fg(DIM))],
    ];

    let mut right = Vec::new();
    if let Some(battery) = &snapshot.battery {
        right.push(Span::styled(
            format!("{} ", battery_icon(battery.fraction)),
            Style::new().fg(heat(1.0 - battery.fraction)),
        ));
        right.push(Span::styled(
            percent(battery.fraction),
            Style::new().fg(TEXT),
        ));
        if let Some(charger) = &battery.charger {
            right.push(Span::styled("  ⏻ ", Style::new().fg(POWER)));
            let input = charger
                .input_watts
                .map(|watts| format!("{watts:.1}"))
                .unwrap_or_else(|| "—".into());
            let rated = charger
                .rated_watts
                .map(|watts| format!("{watts:.0}"))
                .unwrap_or_else(|| "—".into());
            right.push(Span::styled(
                format!("{input}/{rated} W"),
                Style::new().fg(POWER).add_modifier(Modifier::BOLD),
            ));
        }
        right.push(separator());
    }
    if let Some(watts_value) = snapshot.power.chip_watts {
        right.push(Span::styled("⚡ ", Style::new().fg(POWER)));
        right.push(Span::styled(watts(watts_value), Style::new().fg(TEXT)));
        right.push(separator());
    }
    const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    let spinner = SPINNER[(app.anim.now() * 12.0) as usize % SPINNER.len()];
    let state = if app.paused {
        Span::styled(
            " ❚❚ PAUSED ",
            Style::new()
                .fg(Color::Black)
                .bg(POWER)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!(" {spinner} LIVE "),
            Style::new()
                .fg(Color::Black)
                .bg(mix(
                    BATTERY,
                    Color::Rgb(200, 255, 200),
                    0.5 + 0.5 * (app.anim.now() * 3.0).sin(),
                ))
                .add_modifier(Modifier::BOLD),
        )
    };
    right.push(state);
    right.push(Span::styled(
        format!("  {}ms ", app.interval.as_millis()),
        Style::new().fg(DIM),
    ));
    right.extend(bar_spans(
        6,
        if app.paused {
            0.0
        } else {
            app.refresh_progress
        },
        0.0,
        Paint::Tint(CPU),
        BarStyle::Thin,
        app.anim.now(),
    ));
    right.push(Span::raw("  "));
    right.push(Span::styled(
        clock(),
        Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
    ));

    let mut left = flowing_text(" ◉ GLANCE ", &[CPU, GPU, UPLOAD, NPU, CPU], app.anim.now());
    let budget = (inner.width as usize).saturating_sub(line_width(&right) + 1);
    let mut first = true;
    for field in fields {
        let gap = if first { 2 } else { 5 };
        if line_width(&left) + gap + line_width(&field) > budget {
            continue;
        }
        left.push(if first { Span::raw("  ") } else { separator() });
        left.extend(field);
        first = false;
    }
    let columns = Layout::horizontal([
        Constraint::Min(10),
        Constraint::Length(line_width(&right) as u16),
    ])
    .split(inner);
    frame.render_widget(Paragraph::new(Line::from(left)), columns[0]);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        columns[1],
    );
}

fn footer(frame: &mut Frame, area: Rect, app: &App) {
    if let Mode::Filter = app.mode {
        let cursor = if ((app.anim.now() * 2.0) as u64).is_multiple_of(2) || !app.anim.enabled {
            "▌"
        } else {
            " "
        };
        let line = Line::from(vec![
            Span::styled(
                " / search ",
                Style::new()
                    .fg(Color::Black)
                    .bg(POWER)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", app.filter),
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(cursor, Style::new().fg(POWER)),
            Span::styled(
                "    name, bundle ID, path, or PID · Enter keep · Esc clear",
                Style::new().fg(DIM),
            ),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let keys = [
        ("q", "quit"),
        ("?", "help"),
        ("i", "about"),
        ("/", "search"),
        ("↑↓", "select"),
        ("c m p n t", "sort"),
        ("x", "stop"),
        ("space", if app.paused { "resume" } else { "pause" }),
        ("a", "animate"),
    ];
    let mut spans = Vec::new();
    for (key, label) in keys {
        spans.push(Span::styled(
            format!(" {key} "),
            Style::new()
                .fg(Color::Black)
                .bg(CPU)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {label}  "), Style::new().fg(DIM)));
    }
    let toast = app
        .toast
        .as_ref()
        .filter(|toast| toast.shown.elapsed() < Duration::from_secs(4));
    let right = match toast {
        Some(toast) => {
            let age = toast.shown.elapsed().as_secs_f64();
            let fade = if age > 3.0 { 1.0 - (age - 3.0) } else { 1.0 };
            vec![
                Span::styled(" ● ", Style::new().fg(dim(toast.color, fade))),
                Span::styled(
                    format!("{} ", toast.text),
                    Style::new()
                        .fg(dim(TEXT, fade))
                        .add_modifier(Modifier::BOLD),
                ),
            ]
        }
        None if !app.filter.is_empty() => vec![Span::styled(
            format!(" search \"{}\" · Esc clears ", app.filter),
            Style::new().fg(POWER),
        )],
        None => Vec::new(),
    };
    let columns = Layout::horizontal([
        Constraint::Min(10),
        Constraint::Length(line_width(&right) as u16),
    ])
    .split(area);
    frame.render_widget(Paragraph::new(Line::from(spans)), columns[0]);
    frame.render_widget(
        Paragraph::new(Line::from(right)).alignment(Alignment::Right),
        columns[1],
    );
}

fn popup_area(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(4));
    let height = height.min(area.height.saturating_sub(2));
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn help_popup(frame: &mut Frame, area: Rect) {
    let sections: [(&str, &[(&str, &str)]); 3] = [
        (
            "Processes",
            &[
                ("↑ ↓  j k", "move the selection"),
                ("PgUp PgDn", "move ten rows"),
                ("g G  Home End", "jump to the first or last row"),
                ("mouse", "scroll, or click a row to select it"),
                ("/", "search by name, bundle ID, path, or PID"),
                ("c m p n t", "sort by CPU, memory, PID, name, threads"),
                ("r", "reverse the order (same sort key also reverses)"),
                ("x  Delete", "stop the selected process (SIGTERM)"),
                ("X", "force quit the selected process (SIGKILL)"),
            ],
        ),
        (
            "View",
            &[
                ("space", "pause or resume sampling"),
                ("+  -", "refresh faster or slower (250 ms – 5 s)"),
                ("a", "turn animations on or off"),
            ],
        ),
        (
            "General",
            &[
                ("? h F1", "show this help"),
                ("i", "about Glance: author, version, license, source"),
                ("q  Esc  Ctrl-C", "quit"),
            ],
        ),
    ];
    let mut lines = Vec::new();
    for (title, keys) in sections {
        lines.push(Line::from(Span::styled(
            title,
            Style::new().fg(POWER).add_modifier(Modifier::BOLD),
        )));
        for (key, text) in keys {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {key:<16}"),
                    Style::new().fg(CPU).add_modifier(Modifier::BOLD),
                ),
                Span::styled(text.to_string(), Style::new().fg(TEXT)),
            ]));
        }
        lines.push(Line::default());
    }
    lines.push(Line::from(Span::styled(
        "Press any key to close",
        Style::new().fg(DIM),
    )));
    let popup = popup_area(area, 70, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            panel(popup.width, "Keys", "?", CPU, None)
                .style(Style::new().bg(Color::Rgb(18, 19, 27))),
        ),
        popup,
    );
}

fn about_popup(frame: &mut Frame, area: Rect, now: f64) {
    let stops = [CPU, GPU, UPLOAD, NPU, CPU];
    let mut lines: Vec<Line> = about::BANNER
        .iter()
        .enumerate()
        .map(|(row, text)| {
            let count = text.chars().count().max(1) as f64;
            Line::from(
                text.chars()
                    .enumerate()
                    .map(|(column, character)| {
                        let position = ((column as f64 + row as f64 * 2.0) / count - now * 0.3)
                            .rem_euclid(1.0);
                        Span::styled(
                            character.to_string(),
                            Style::new()
                                .fg(gradient(&stops, position))
                                .add_modifier(Modifier::BOLD),
                        )
                    })
                    .collect::<Vec<_>>(),
            )
            .alignment(Alignment::Center)
        })
        .collect();
    lines.push(Line::default());
    lines.push(
        Line::from(vec![
            Span::styled(
                about::NAME,
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" — {}", about::DESCRIPTION), Style::new().fg(DIM)),
        ])
        .alignment(Alignment::Center),
    );
    lines.push(Line::default());
    for (label, value) in about::details() {
        let color = match label {
            "Author" => PROCESS,
            "Source" => CPU,
            "Version" => BATTERY,
            _ => TEXT,
        };
        lines.push(Line::from(vec![
            Span::styled(format!("  {label:<12}"), Style::new().fg(DIM)),
            Span::styled(value, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        ]));
    }
    lines.push(Line::default());
    lines.push(
        Line::from(Span::styled("Press any key to close", Style::new().fg(DIM)))
            .alignment(Alignment::Center),
    );
    let popup = popup_area(area, 74, lines.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            panel(popup.width, "About", "i", GPU, None)
                .style(Style::new().bg(Color::Rgb(18, 19, 27))),
        ),
        popup,
    );
}

fn confirm_popup(frame: &mut Frame, area: Rect, pid: i32, name: &str, force: bool) {
    let (verb, signal) = if force {
        ("Force quit", "SIGKILL — unsaved work is lost")
    } else {
        ("Stop", "SIGTERM — the process can clean up first")
    };
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{verb} "), Style::new().fg(TEXT)),
            Span::styled(
                name.to_string(),
                Style::new().fg(PROCESS).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" (pid {pid})?"), Style::new().fg(TEXT)),
        ]),
        Line::from(Span::styled(signal, Style::new().fg(DIM))),
        Line::default(),
        Line::from(vec![
            Span::styled(
                " y ",
                Style::new()
                    .fg(Color::Black)
                    .bg(ALERT)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" {verb}    "), Style::new().fg(TEXT)),
            Span::styled(
                " any other key ",
                Style::new()
                    .fg(Color::Black)
                    .bg(DIM)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" cancel", Style::new().fg(TEXT)),
        ]),
    ];
    let popup = popup_area(area, 60, 6);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines).block(
            panel(popup.width, verb, "!", ALERT, None)
                .style(Style::new().bg(Color::Rgb(24, 16, 22))),
        ),
        popup,
    );
}

// MARK: - CPU

fn cpu_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let cpu = &snapshot.cpu;
    let right = badge(
        [
            Some(percent(cpu.utilization)),
            cpu.power_watts.map(watts),
            cpu.temperature_c.map(celsius),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join("  ·  "),
        heat(cpu.utilization).max_bright(),
    );
    let hot = cpu.utilization > 0.85 || cpu.temperature_c.is_some_and(|value| value > 95.0);
    let block = panel(area.width, "CPU", "▣", alert(app, CPU, hot), Some(right));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let cluster_rows = cpu.clusters.len() as u16;
    let spare = inner.height.saturating_sub(cluster_rows + 1);
    let rows_for = |cell: u16| {
        let columns = (inner.width / cell).max(1) as usize;
        cpu.cores.len().div_ceil(columns) as u16
    };
    let (cell_width, core_rows) = if rows_for(22) + 3 <= spare {
        (22, rows_for(22))
    } else {
        (13, rows_for(13).min(spare))
    };
    let chart_height = spare.saturating_sub(core_rows).min(12);
    let parts = Layout::vertical([
        Constraint::Length(chart_height),
        Constraint::Length(cluster_rows),
        Constraint::Length(1),
        Constraint::Length(core_rows),
        Constraint::Min(0),
    ])
    .split(inner);

    if chart_height >= 3 {
        line_chart(frame, parts[0], &[(&app.history.cpu, CPU)], 1.0, true);
    }
    for (index, cluster) in cpu.clusters.iter().enumerate() {
        let row = Rect {
            y: parts[1].y + index as u16,
            height: 1,
            ..parts[1]
        };
        cluster_row(frame, row, app, cluster);
    }
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(parts[2].width as usize),
            Style::new().fg(FAINT),
        ))),
        parts[2],
    );
    core_grid(frame, parts[3], app, snapshot, cell_width);
}

fn cluster_row(frame: &mut Frame, area: Rect, app: &App, cluster: &Cluster) {
    let name = if cluster.name.is_empty() {
        cluster.kind.clone()
    } else {
        cluster.name.clone()
    };
    let color = level_color(&cluster.kind);
    let columns = Layout::horizontal([Constraint::Length(46), Constraint::Min(10)]).split(area);
    let text = Line::from(vec![
        Span::styled("● ", Style::new().fg(color)),
        Span::styled(
            format!("{name:<12}"),
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("×{:<3}", cluster.core_count), Style::new().fg(DIM)),
        Span::styled(
            format!(
                "{:>9}",
                cluster
                    .frequency_hz
                    .map(hertz)
                    .unwrap_or_else(|| "—".into())
            ),
            Style::new().fg(color),
        ),
        Span::styled(
            format!(
                "{:>9}",
                cluster.power_watts.map(watts).unwrap_or_else(|| "—".into())
            ),
            Style::new().fg(POWER),
        ),
    ]);
    frame.render_widget(Paragraph::new(text), columns[0]);
    meter(
        frame,
        columns[1],
        app,
        &format!("cluster.{}", cluster.kind),
        cluster.utilization,
        Label::Percent(""),
        Paint::Heat,
        BarStyle::Thin,
    );
}

fn core_grid(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot, cell_width: u16) {
    let cores = &snapshot.cpu.cores;
    if cores.is_empty() || area.height == 0 {
        return;
    }
    let rows = area.height as usize;
    let mut columns = cores.len().div_ceil(rows).max(1);
    while columns > 1 && area.width as usize / columns < cell_width as usize {
        columns -= 1;
    }
    let per_column = cores.len().div_ceil(columns);
    let column_areas = Layout::horizontal(vec![Constraint::Ratio(1, columns as u32); columns])
        .spacing(if cell_width >= 22 { 2 } else { 1 })
        .split(area);
    for (column, column_area) in column_areas.iter().enumerate() {
        for row in 0..per_column.min(rows) {
            let Some(core) = cores.get(column * per_column + row) else {
                continue;
            };
            let cell = Rect {
                y: column_area.y + row as u16,
                height: 1,
                ..*column_area
            };
            let parts = Layout::horizontal([
                Constraint::Length(4),
                Constraint::Min(2),
                Constraint::Length(if cell_width >= 22 { 6 } else { 4 }),
            ])
            .split(cell);
            let level_kind = snapshot
                .cpu
                .clusters
                .get(core.level as usize)
                .map(|cluster| cluster.kind.as_str())
                .unwrap_or("");
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("C{:<2}", core.id),
                    Style::new().fg(level_color(level_kind)),
                )),
                parts[0],
            );
            let values = app.history.cores.get(&core.id).cloned().unwrap_or_default();
            let (shown, _) = app
                .anim
                .ease(&format!("core.{}", core.id), core.utilization);
            heat_sparkline(frame, parts[1], &values, 1.0);
            frame.render_widget(
                Paragraph::new(Span::styled(
                    format!("{:>3.0}%", shown * 100.0),
                    Style::new().fg(heat(shown)).add_modifier(Modifier::BOLD),
                ))
                .alignment(Alignment::Right),
                parts[2],
            );
        }
    }
}

// MARK: - GPU, power, thermal

fn gpu_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let gpu = &snapshot.gpu;
    let utilization = gpu.utilization.unwrap_or(0.0);
    let block = panel(
        area.width,
        "GPU",
        "◈",
        GPU,
        Some(badge(
            gpu.utilization.map(percent).unwrap_or_else(|| "n/a".into()),
            GPU,
        )),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parts = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    line_chart(frame, parts[0], &[(&app.history.gpu, GPU)], 1.0, false);
    meter(
        frame,
        parts[1],
        app,
        "gpu.load",
        utilization,
        Label::Percent("load "),
        Paint::Tint(GPU),
        BarStyle::Thick,
    );
    let stats = Line::from(
        vec![
            stat(
                "mem",
                gpu.memory_bytes.map(bytes).unwrap_or_else(|| "—".into()),
                GPU,
            ),
            stat(
                "clock",
                gpu.frequency_hz.map(hertz).unwrap_or_else(|| "—".into()),
                GPU,
            ),
            stat(
                "power",
                gpu.power_watts.map(watts).unwrap_or_else(|| "—".into()),
                POWER,
            ),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>(),
    );
    frame.render_widget(Paragraph::new(stats), parts[2]);
}

fn power_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let power = &snapshot.power;
    let npu_state = match snapshot.npu.active {
        Some(true) => Span::styled(
            " NPU ACTIVE ",
            Style::new()
                .fg(Color::Black)
                .bg(mix(
                    NPU,
                    Color::Rgb(220, 255, 240),
                    0.5 + 0.5 * (app.anim.now() * 4.0).sin(),
                ))
                .add_modifier(Modifier::BOLD),
        ),
        Some(false) => Span::styled(
            " NPU IDLE ",
            Style::new().fg(NPU).add_modifier(Modifier::BOLD),
        ),
        None => Span::styled(" NPU n/a ", Style::new().fg(DIM)),
    };
    let block = panel(
        area.width,
        "Power · NPU · Thermal",
        "⚡",
        POWER,
        Some(badge(
            power.chip_watts.map(watts).unwrap_or_else(|| "n/a".into()),
            POWER,
        )),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let columns = Layout::horizontal([Constraint::Percentage(55), Constraint::Percentage(45)])
        .spacing(1)
        .split(inner);

    let values = [
        ("CPU", power.cpu_watts, CPU),
        ("GPU", power.gpu_watts, GPU),
        ("NPU", power.npu_watts, NPU),
        ("MEM", power.memory_watts, MEMORY),
    ];
    let bars: Vec<Bar> = values
        .iter()
        .map(|(label, value, color)| {
            let (eased, _) = app
                .anim
                .ease(&format!("power.{label}"), value.unwrap_or(0.0));
            let milliwatts = (eased * 1000.0) as u64;
            Bar::default()
                .value(milliwatts)
                .text_value(
                    value
                        .map(|_| format!("{eased:.1}"))
                        .unwrap_or_else(|| "—".into()),
                )
                .label(Line::from(*label))
                .style(Style::new().fg(*color))
                .value_style(
                    Style::new()
                        .fg(Color::Black)
                        .bg(*color)
                        .add_modifier(Modifier::BOLD),
                )
        })
        .collect();
    let bar_width = ((columns[0].width.saturating_sub(3)) / 4).clamp(3, 9);
    let chart = BarChart::default()
        .data(BarGroup::default().bars(&bars))
        .bar_width(bar_width)
        .bar_gap(1)
        .label_style(Style::new().fg(DIM));
    frame.render_widget(chart, columns[0]);

    let right = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
    ])
    .split(columns[1]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            npu_state,
            Span::styled(
                format!(
                    " {}",
                    snapshot
                        .npu
                        .power_watts
                        .map(watts)
                        .unwrap_or_else(|| "—".into())
                ),
                Style::new().fg(TEXT),
            ),
        ])),
        right[0],
    );
    let temperature = snapshot.cpu.temperature_c;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("die temp ", Style::new().fg(DIM)),
            Span::styled(
                temperature.map(celsius).unwrap_or_else(|| "—".into()),
                Style::new()
                    .fg(heat(temperature.unwrap_or(0.0) / 105.0))
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        right[1],
    );
    meter(
        frame,
        right[2],
        app,
        "thermal.die",
        temperature.unwrap_or(0.0) / 105.0,
        Label::None,
        Paint::Heat,
        BarStyle::Thick,
    );
    frame.render_widget(
        Paragraph::new(Span::styled("chip power", Style::new().fg(DIM))),
        right[3],
    );
    let peak = app.history.chip.iter().copied().fold(1.0, f64::max);
    colored_sparkline(frame, right[4], &app.history.chip, peak, POWER);
}

// MARK: - Memory, network, disk

fn memory_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let memory = &snapshot.memory;
    let total = memory.total_bytes.max(1) as f64;
    let used = memory.used_bytes as f64 / total;
    let pressure_color = match memory.pressure.as_str() {
        "critical" => Color::Rgb(255, 80, 90),
        "warning" => POWER,
        _ => BATTERY,
    };
    let block = panel(
        area.width,
        "Memory",
        "▤",
        alert(app, MEMORY, memory.pressure != "normal"),
        Some(badge(
            format!(
                "{} / {}",
                bytes(memory.used_bytes),
                bytes(memory.total_bytes)
            ),
            MEMORY,
        )),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let parts: [(&str, u64, Color); 5] = [
        ("wired", memory.wired_bytes, Color::Rgb(255, 95, 120)),
        ("active", memory.active_bytes, MEMORY),
        ("compressed", memory.compressed_bytes, GPU),
        ("inactive", memory.inactive_bytes, CPU),
        ("free", memory.free_bytes, Color::Rgb(80, 85, 100)),
    ];
    let rows = Layout::vertical(vec![Constraint::Length(1); 8]).split(inner);
    meter(
        frame,
        rows[0],
        app,
        "memory.used",
        used,
        Label::Percent("used "),
        Paint::Heat,
        BarStyle::Thick,
    );
    let shares: Vec<(f64, f64)> = parts
        .iter()
        .map(|(label, value, _)| {
            app.anim
                .ease(&format!("memory.{label}"), *value as f64 / total)
        })
        .collect();

    let width = rows[1].width as usize;
    let mut stacked = Vec::new();
    let mut drawn = 0usize;
    for (index, (_, _, color)) in parts.iter().enumerate() {
        let cells = if index == parts.len() - 1 {
            width.saturating_sub(drawn)
        } else {
            (shares[index].0 * width as f64).round() as usize
        };
        let cells = cells.min(width.saturating_sub(drawn));
        drawn += cells;
        stacked.push(Span::styled("█".repeat(cells), Style::new().fg(*color)));
    }
    frame.render_widget(Paragraph::new(Line::from(stacked)), rows[1]);

    for (index, (label, value, color)) in parts.iter().enumerate() {
        let row = rows[2 + index];
        let (share, peak) = shares[index];
        let columns = Layout::horizontal([Constraint::Length(24), Constraint::Min(4)]).split(row);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("■ ", Style::new().fg(*color)),
                Span::styled(format!("{label:<11}"), Style::new().fg(DIM)),
                Span::styled(format!("{:>9}", bytes(*value)), Style::new().fg(TEXT)),
            ])),
            columns[0],
        );
        frame.render_widget(
            Paragraph::new(Line::from(bar_spans(
                columns[1].width as usize,
                share,
                peak,
                Paint::Tint(*color),
                BarStyle::Thin,
                app.anim.now(),
            ))),
            columns[1],
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("swap ", Style::new().fg(DIM)),
            Span::styled(bytes(memory.swap_bytes), Style::new().fg(TEXT)),
            Span::styled("   pressure ", Style::new().fg(DIM)),
            Span::styled(
                format!(" {} ", memory.pressure.to_uppercase()),
                Style::new()
                    .fg(Color::Black)
                    .bg(pressure_color)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[7],
    );
}

fn network_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let network = &snapshot.network;
    let history = &app.history;
    let block = panel(area.width, "Network", "⇅", NETWORK, None);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parts = Layout::vertical([
        Constraint::Min(2),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);
    let peak = history
        .download
        .iter()
        .chain(history.upload.iter())
        .copied()
        .fold(1.0, f64::max);
    line_chart(
        frame,
        parts[0],
        &[(&history.download, NETWORK), (&history.upload, UPLOAD)],
        peak,
        false,
    );
    let rates = [
        ("▼", "net.down", network.download_bytes_per_second, NETWORK),
        ("▲", "net.up", network.upload_bytes_per_second, UPLOAD),
    ];
    for (row, (arrow, key, rate, color)) in rates.into_iter().enumerate() {
        let columns =
            Layout::horizontal([Constraint::Length(14), Constraint::Min(4)]).split(parts[1 + row]);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{arrow} "),
                    Style::new().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    bytes_per_second(rate),
                    Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
                ),
            ])),
            columns[0],
        );
        meter(
            frame,
            columns[1],
            app,
            key,
            rate / peak,
            Label::None,
            Paint::Tint(color),
            BarStyle::Thin,
        );
    }
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("peak  ", Style::new().fg(DIM)),
            Span::styled(
                format!("▼ {:<12}", bytes_per_second(history.peak_download)),
                Style::new().fg(NETWORK),
            ),
            Span::styled(
                format!("▲ {}", bytes_per_second(history.peak_upload)),
                Style::new().fg(UPLOAD),
            ),
        ])),
        parts[3],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("total ", Style::new().fg(DIM)),
            Span::styled(
                format!("▼ {:<12}", bytes(history.total_download as u64)),
                Style::new().fg(NETWORK),
            ),
            Span::styled(
                format!("▲ {}", bytes(history.total_upload as u64)),
                Style::new().fg(UPLOAD),
            ),
        ])),
        parts[4],
    );
}

fn disk_panel(frame: &mut Frame, area: Rect, app: &App, snapshot: &Snapshot) {
    let disk = &snapshot.disk;
    let used = disk
        .total_bytes
        .saturating_sub(disk.free_bytes.min(disk.total_bytes));
    let fraction = if disk.total_bytes > 0 {
        used as f64 / disk.total_bytes as f64
    } else {
        0.0
    };
    let block = panel(
        area.width,
        "Disk · Battery",
        "◍",
        DISK,
        Some(badge(format!("{} free", bytes(disk.free_bytes)), DISK)),
    );
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows =
        Layout::vertical(vec![Constraint::Length(1); inner.height.max(8) as usize]).split(inner);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("/ ", Style::new().fg(DISK).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{} / {}", bytes(used), bytes(disk.total_bytes)),
                Style::new().fg(TEXT),
            ),
        ])),
        rows[0],
    );
    meter(
        frame,
        rows[1],
        app,
        "disk.used",
        fraction,
        Label::Percent(""),
        Paint::Tint(DISK),
        BarStyle::Thick,
    );
    let rate_peak = app
        .history
        .read
        .iter()
        .chain(app.history.write.iter())
        .copied()
        .fold(1.0, f64::max);
    labeled_spark(
        frame,
        rows[2],
        "read ",
        disk.read_bytes_per_second
            .map(bytes_per_second)
            .unwrap_or_else(|| "—".into()),
        &app.history.read,
        rate_peak,
        BATTERY,
    );
    labeled_spark(
        frame,
        rows[3],
        "write",
        disk.write_bytes_per_second
            .map(bytes_per_second)
            .unwrap_or_else(|| "—".into()),
        &app.history.write,
        rate_peak,
        UPLOAD,
    );

    if let Some(battery) = &snapshot.battery {
        let charger = battery.charger.as_ref();
        // With a charger and only eight lines, the divider gives way to the charger row.
        let roomy = inner.height >= 10;
        let mut line = 4;
        if charger.is_none() || roomy {
            frame.render_widget(
                Paragraph::new(Span::styled(
                    "─".repeat(rows[line].width as usize),
                    Style::new().fg(FAINT),
                )),
                rows[line],
            );
            line += 1;
        }
        let level = heat(1.0 - battery.fraction);
        let status = match (battery.charging, battery.external_power) {
            (true, _) => {
                let flash = 0.5 + 0.5 * (app.anim.now() * 4.0).sin();
                Span::styled(
                    "  ⚡ charging",
                    Style::new()
                        .fg(mix(POWER, Color::Rgb(255, 255, 220), flash))
                        .add_modifier(Modifier::BOLD),
                )
            }
            (false, true) => Span::styled("  ● on AC", Style::new().fg(BATTERY)),
            (false, false) => Span::styled("  ○ on battery", Style::new().fg(POWER)),
        };
        let mut title = vec![
            Span::styled(
                format!("{} ", battery_icon(battery.fraction)),
                Style::new().fg(level),
            ),
            Span::styled(
                "Battery ",
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                percent(battery.fraction),
                Style::new().fg(level).add_modifier(Modifier::BOLD),
            ),
            status,
        ];
        if let Some(minutes) = battery.minutes_remaining {
            let suffix = if battery.charging { "to full" } else { "left" };
            title.push(Span::styled(
                format!(" · {}h {:02}m {suffix}", minutes / 60, minutes % 60),
                Style::new().fg(DIM),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(title)), rows[line]);
        line += 1;
        let paint = if battery.charging {
            Paint::Charge
        } else {
            Paint::Tint(level)
        };
        meter(
            frame,
            rows[line],
            app,
            "battery",
            battery.fraction,
            Label::None,
            paint,
            BarStyle::Thick,
        );
        line += 1;
        if let Some(charger) = charger {
            charger_row(frame, rows[line], app, charger, battery.charging);
            line += 1;
            if roomy {
                charger_details(frame, rows[line], charger);
                line += 1;
            }
        }
        let mut health = vec![
            Span::styled("health ", Style::new().fg(DIM)),
            Span::styled(
                battery.health.map(percent).unwrap_or_else(|| "—".into()),
                Style::new().fg(heat(1.0 - battery.health.unwrap_or(1.0)).max_bright()),
            ),
            Span::styled("   cycles ", Style::new().fg(DIM)),
            Span::styled(
                battery
                    .cycles
                    .map(|count| count.to_string())
                    .unwrap_or_else(|| "—".into()),
                Style::new().fg(TEXT),
            ),
        ];
        if let Some(flow) = battery.battery_watts.filter(|flow| flow.abs() >= 0.05) {
            let (text, color) = if flow > 0.0 {
                (format!("   ▲ +{}", watts(flow)), BATTERY)
            } else {
                (format!("   ▼ {}", watts(flow.abs())), POWER)
            };
            health.push(Span::styled(
                text,
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ));
        }
        frame.render_widget(Paragraph::new(Line::from(health)), rows[line]);
    }
}

/// "In 14.9 W" with a bar of input against the adapter's rating, then the rating.
fn charger_row(frame: &mut Frame, area: Rect, app: &App, charger: &Charger, charging: bool) {
    let input = charger.input_watts;
    let rated = charger.rated_watts;
    let rating = rated
        .map(|watts| format!("/ {watts:.0} W"))
        .unwrap_or_else(|| "/ —".into());
    let columns = Layout::horizontal([
        Constraint::Length(13),
        Constraint::Min(3),
        Constraint::Length(rating.chars().count() as u16 + 1),
    ])
    .split(area);
    let icon = if charger.wireless { "◎" } else { "⏻" };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!("{icon} in "),
                Style::new().fg(POWER).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                input.map(watts).unwrap_or_else(|| "—".into()),
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
        ])),
        columns[0],
    );
    let share = match (input, rated) {
        (Some(input), Some(rated)) if rated > 0.0 => input / rated,
        _ => 0.0,
    };
    let paint = if charging {
        Paint::Charge
    } else {
        Paint::Tint(POWER)
    };
    meter(
        frame,
        columns[1],
        app,
        "charger.input",
        share,
        Label::None,
        paint,
        BarStyle::Thin,
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            rating,
            Style::new().fg(POWER).add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Right),
        columns[2],
    );
}

/// The adapter's negotiated voltage and current next to what is flowing now.
fn charger_details(frame: &mut Frame, area: Rect, charger: &Charger) {
    let pair = |volts: Option<f64>, amps: Option<f64>| match (volts, amps) {
        (Some(volts), Some(amps)) => format!("{volts:.1}V {amps:.2}A"),
        (Some(volts), None) => format!("{volts:.1}V"),
        (None, Some(amps)) => format!("{amps:.2}A"),
        (None, None) => "—".into(),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("adapter ", Style::new().fg(DIM)),
            Span::styled(
                pair(charger.rated_volts, charger.rated_amps),
                Style::new().fg(TEXT),
            ),
            Span::styled("   now ", Style::new().fg(DIM)),
            Span::styled(
                pair(charger.input_volts, charger.input_amps),
                Style::new().fg(POWER),
            ),
        ])),
        area,
    );
}

// MARK: - Processes

fn process_panel(frame: &mut Frame, area: Rect, app: &mut App, snapshot: &Snapshot) {
    app.process_area = area;
    let visible = app.visible(snapshot);
    let index = app
        .selected_pid
        .and_then(|pid| visible.iter().position(|process| process.pid == pid))
        .or_else(|| {
            app.table
                .selected()
                .map(|index| index.min(visible.len().saturating_sub(1)))
        });
    match index.filter(|_| !visible.is_empty()) {
        Some(index) => {
            app.table.select(Some(index));
            app.selected_pid = Some(visible[index].pid);
        }
        None => app.table.select(None),
    }
    let arrow = if app.sort.descending_by_default() != app.reversed {
        "▼"
    } else {
        "▲"
    };
    let mut summary = if app.filter.is_empty() {
        format!("{} processes", visible.len())
    } else {
        format!(
            "{} of {} · \"{}\"",
            visible.len(),
            snapshot.processes.len(),
            app.filter
        )
    };
    summary.push_str(&format!(" · {} {arrow}", app.sort.label()));
    let selected = app.table.selected().and_then(|index| visible.get(index));
    let mut block = panel(
        area.width,
        "Processes",
        "≡",
        PROCESS,
        Some(badge(summary, PROCESS)),
    );
    if let Some(process) = selected {
        block = block.title_bottom(Line::from(vec![
            Span::styled(" ▸ ", Style::new().fg(PROCESS)),
            Span::styled(
                format!("{} ", process.name),
                Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("pid {}  ", process.pid), Style::new().fg(DIM)),
            Span::styled(format!("{} ", process.identifier), Style::new().fg(CPU)),
            Span::styled("─ x stop · X force ", Style::new().fg(DIM)),
        ]));
    }
    let peak = visible
        .iter()
        .map(|process| process.cpu)
        .fold(0.01, f64::max);
    let total_memory = snapshot.memory.total_bytes.max(1) as f64;
    let now = app.anim.now();
    let rows: Vec<Row> = visible
        .iter()
        .map(|process| {
            let (share, top) = app
                .anim
                .ease(&format!("process.{}", process.pid), process.cpu / peak);
            let mut bar = bar_spans(10, share, top, Paint::Heat, BarStyle::Thick, now);
            bar.push(Span::styled(
                format!(" {:>6}", percent(process.cpu)),
                Style::new()
                    .fg(heat(process.cpu.min(1.0)).max_bright())
                    .add_modifier(Modifier::BOLD),
            ));
            let memory_share = process.memory_bytes as f64 / total_memory;
            Row::new(vec![
                Cell::from(Span::styled(process.pid.to_string(), Style::new().fg(DIM))),
                Cell::from(Span::styled(
                    process.name.clone(),
                    Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
                )),
                Cell::from(Line::from(bar)),
                Cell::from(Span::styled(
                    bytes(process.memory_bytes),
                    Style::new().fg(heat(memory_share * 8.0)),
                )),
                Cell::from(Span::styled(
                    process.threads.to_string(),
                    Style::new().fg(DIM),
                )),
                Cell::from(Span::styled(
                    process.identifier.clone(),
                    Style::new().fg(Color::Rgb(130, 150, 190)),
                )),
            ])
        })
        .collect();
    let columns = [
        ("PID", Some(SortKey::Pid)),
        ("NAME", Some(SortKey::Name)),
        ("CPU", Some(SortKey::Cpu)),
        ("MEMORY", Some(SortKey::Memory)),
        ("THR", Some(SortKey::Threads)),
        ("BUNDLE ID / PATH", None),
    ];
    let header = Row::new(columns.map(|(title, key)| {
        let active = key == Some(app.sort);
        let text = if active {
            format!("{title} {arrow}")
        } else {
            title.to_string()
        };
        let style = if active {
            Style::new()
                .fg(Color::Black)
                .bg(PROCESS)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(PROCESS).add_modifier(Modifier::BOLD)
        };
        Cell::from(Span::styled(text, style))
    }));
    let empty = rows.is_empty();
    let pulse = 0.5 + 0.5 * (now * 3.0).sin();
    let table = Table::new(
        rows,
        [
            Constraint::Length(7),
            Constraint::Length(22),
            Constraint::Length(18),
            Constraint::Length(10),
            Constraint::Length(5),
            Constraint::Min(20),
        ],
    )
    .header(header)
    .block(block)
    .column_spacing(1)
    .row_highlight_style(
        Style::new()
            .bg(mix(Color::Rgb(40, 34, 64), Color::Rgb(70, 50, 100), pulse))
            .add_modifier(Modifier::BOLD),
    )
    .highlight_symbol(Span::styled("▶ ", Style::new().fg(PROCESS)));
    frame.render_stateful_widget(table, area, &mut app.table);
    if empty {
        let message = Rect {
            y: area.y + 3,
            height: 1,
            ..area
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("No process matches \"{}\"", app.filter),
                Style::new().fg(DIM),
            ))
            .alignment(Alignment::Center),
            message,
        );
    }
}

// MARK: - Widgets

fn line_chart(
    frame: &mut Frame,
    area: Rect,
    series: &[(&VecDeque<f64>, Color)],
    max: f64,
    labels: bool,
) {
    if area.height < 2 || area.width < 4 {
        return;
    }
    let points: Vec<Vec<(f64, f64)>> = series
        .iter()
        .map(|(values, _)| {
            let offset = HISTORY.saturating_sub(values.len());
            values
                .iter()
                .enumerate()
                .map(|(index, value)| ((offset + index) as f64, *value))
                .collect()
        })
        .collect();
    let datasets: Vec<Dataset> = points
        .iter()
        .zip(series)
        .map(|(data, (_, color))| {
            Dataset::default()
                .marker(Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::new().fg(*color))
                .data(data)
        })
        .collect();
    let mut y_axis = Axis::default()
        .bounds([0.0, max.max(1e-9)])
        .style(Style::new().fg(FAINT));
    if labels {
        y_axis = y_axis
            .labels(["0", "50", "100"].map(|label| Span::styled(label, Style::new().fg(DIM))));
    }
    let chart = Chart::new(datasets)
        .x_axis(Axis::default().bounds([0.0, (HISTORY - 1) as f64]))
        .y_axis(y_axis);
    frame.render_widget(chart, area);
}

#[derive(Clone, Copy)]
enum Paint {
    /// Green to red along the bar, so a fuller bar runs hotter.
    Heat,
    /// One hue from dim to bright.
    Tint(Color),
    /// Green with energy flowing toward the head.
    Charge,
}

#[derive(Clone, Copy, PartialEq)]
enum BarStyle {
    /// A heavy line with a half-cell head.
    Thin,
    /// Solid blocks on a dark track, with eighth-cell precision.
    Thick,
}

enum Label<'a> {
    None,
    /// The eased value as a percentage after a prefix, so the number counts with the bar.
    Percent(&'a str),
}

const EIGHTHS: [&str; 9] = ["", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

/// Eases `target` under `key`, then draws the bar and its label.
#[allow(clippy::too_many_arguments)]
fn meter(
    frame: &mut Frame,
    area: Rect,
    app: &App,
    key: &str,
    target: f64,
    label: Label,
    paint: Paint,
    style: BarStyle,
) {
    let (value, peak) = app.anim.ease(key, target.clamp(0.0, 1.0));
    let (text, color) = match label {
        Label::None => (String::new(), TEXT),
        Label::Percent(prefix) => (
            format!("{prefix}{}", percent(value)),
            label_color(paint, value),
        ),
    };
    let width = if text.is_empty() {
        0
    } else {
        text.chars().count() as u16 + 1
    };
    let columns = Layout::horizontal([Constraint::Min(3), Constraint::Length(width)]).split(area);
    let spans = bar_spans(
        columns[0].width as usize,
        value,
        peak,
        paint,
        style,
        app.anim.now(),
    );
    frame.render_widget(Paragraph::new(Line::from(spans)), columns[0]);
    if !text.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                text,
                Style::new().fg(color).add_modifier(Modifier::BOLD),
            ))
            .alignment(Alignment::Right),
            columns[1],
        );
    }
}

fn label_color(paint: Paint, value: f64) -> Color {
    match paint {
        Paint::Heat => heat(value).max_bright(),
        Paint::Tint(color) => color,
        Paint::Charge => BATTERY,
    }
}

fn paint_color(paint: Paint, position: f64) -> Color {
    match paint {
        Paint::Heat => heat(position),
        Paint::Tint(color) => gradient(&[dim(color, 0.45), color, color.max_bright()], position),
        Paint::Charge => gradient(
            &[Color::Rgb(40, 170, 90), BATTERY, Color::Rgb(190, 255, 180)],
            position,
        ),
    }
}

/// The bar as styled cells: gradient fill, a shimmer that sweeps across it, a glow
/// past the head, a peak marker that holds and falls, and a pulse near full scale.
fn bar_spans(
    width: usize,
    value: f64,
    peak: f64,
    paint: Paint,
    style: BarStyle,
    now: f64,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let value = value.clamp(0.0, 1.0);
    let exact = value * width as f64;
    let full = exact.floor() as usize;
    let partial = ((exact - full as f64) * 8.0).round() as usize;
    let (full, partial) = if partial == 8 {
        (full + 1, 0)
    } else {
        (full, partial)
    };
    let peak_cell = ((peak.clamp(0.0, 1.0) * width as f64).ceil() as usize).saturating_sub(1);
    let show_peak = peak > value + 0.015 && peak_cell >= full;
    let filled = full as f64 + partial as f64 / 8.0;
    let sweep = (now / 2.4).fract() * (filled + 12.0) - 6.0;
    let pulse = if value >= 0.85 {
        0.12 + 0.18 * (0.5 + 0.5 * (now * 7.0).sin())
    } else {
        0.0
    };
    let white = Color::Rgb(255, 255, 255);
    let tone = |index: usize| {
        let position = index as f64 / (width.max(2) - 1) as f64;
        let mut color = paint_color(paint, position);
        let glint = match paint {
            Paint::Charge => ((index as f64 * 0.7 - now * 7.0).sin() * 0.5 + 0.5).powi(6) * 0.5,
            _ => (1.0 - (index as f64 - sweep).abs() / 5.0).max(0.0).powi(2) * 0.55,
        };
        color = mix(color, white, glint + pulse);
        color
    };
    let track_line = Color::Rgb(48, 52, 66);
    (0..width)
        .map(|index| match style {
            BarStyle::Thick => {
                if index < full {
                    Span::styled("█", Style::new().fg(tone(index)).bg(TRACK))
                } else if index == full && partial > 0 {
                    Span::styled(EIGHTHS[partial], Style::new().fg(tone(index)).bg(TRACK))
                } else if show_peak && index == peak_cell {
                    Span::styled(
                        "▕",
                        Style::new()
                            .fg(paint_color(paint, peak).max_bright())
                            .bg(TRACK),
                    )
                } else if index == full && value > 0.0 {
                    Span::styled(" ", Style::new().bg(mix(TRACK, tone(index), 0.22)))
                } else {
                    Span::styled(" ", Style::new().bg(TRACK))
                }
            }
            BarStyle::Thin => {
                if index < full {
                    Span::styled("━", Style::new().fg(tone(index)))
                } else if index == full && partial >= 4 {
                    Span::styled("╸", Style::new().fg(tone(index)))
                } else if show_peak && index == peak_cell {
                    Span::styled("┃", Style::new().fg(paint_color(paint, peak).max_bright()))
                } else {
                    Span::styled("━", Style::new().fg(track_line))
                }
            }
        })
        .collect()
}

/// Mixes toward the alert color in a slow throb while `active`.
fn alert(app: &App, color: Color, active: bool) -> Color {
    if !active {
        return color;
    }
    mix(color, ALERT, 0.35 + 0.35 * (app.anim.now() * 5.0).sin())
}

fn heat_sparkline(frame: &mut Frame, area: Rect, values: &VecDeque<f64>, max: f64) {
    let width = area.width as usize;
    let start = values.len().saturating_sub(width);
    let bars: Vec<SparklineBar> = values
        .iter()
        .skip(start)
        .map(|value| {
            SparklineBar::from((value / max * 100.0).round() as u64)
                .style(Some(Style::new().fg(heat(*value))))
        })
        .collect();
    frame.render_widget(
        Sparkline::default()
            .data(bars)
            .max(100)
            .style(Style::new().bg(Color::Rgb(24, 26, 34))),
        area,
    );
}

fn colored_sparkline(
    frame: &mut Frame,
    area: Rect,
    values: &VecDeque<f64>,
    max: f64,
    color: Color,
) {
    let width = area.width as usize;
    let start = values.len().saturating_sub(width);
    let bars: Vec<SparklineBar> =
        values
            .iter()
            .skip(start)
            .map(|value| {
                let ratio = (value / max.max(1e-9)).clamp(0.0, 1.0);
                SparklineBar::from((ratio * 100.0).round() as u64)
                    .style(Some(Style::new().fg(mix(dim(color, 0.5), color, ratio))))
            })
            .collect();
    frame.render_widget(Sparkline::default().data(bars).max(100), area);
}

fn labeled_spark(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: String,
    values: &VecDeque<f64>,
    max: f64,
    color: Color,
) {
    let columns = Layout::horizontal([
        Constraint::Length(6),
        Constraint::Min(4),
        Constraint::Length(12),
    ])
    .split(area);
    frame.render_widget(
        Paragraph::new(Span::styled(label.to_string(), Style::new().fg(DIM))),
        columns[0],
    );
    colored_sparkline(frame, columns[1], values, max, color);
    frame.render_widget(
        Paragraph::new(Span::styled(value, Style::new().fg(TEXT))).alignment(Alignment::Right),
        columns[2],
    );
}

fn stat(label: &str, value: String, color: Color) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{label} "), Style::new().fg(DIM)),
        Span::styled(value, Style::new().fg(color).add_modifier(Modifier::BOLD)),
        Span::raw("   "),
    ]
}

fn separator() -> Span<'static> {
    Span::styled("  │  ", Style::new().fg(FAINT))
}

fn line_width(spans: &[Span]) -> usize {
    spans
        .iter()
        .map(|span| span.content.chars().count())
        .sum::<usize>()
        + 1
}

/// Bold text whose gradient drifts along the letters over time.
fn flowing_text(text: &str, stops: &[Color], now: f64) -> Vec<Span<'static>> {
    let characters: Vec<char> = text.chars().collect();
    let count = characters.len().max(1) as f64;
    characters
        .iter()
        .enumerate()
        .map(|(index, character)| {
            let position = (index as f64 / count - now * 0.35).rem_euclid(1.0);
            Span::styled(
                character.to_string(),
                Style::new()
                    .fg(gradient(stops, position))
                    .add_modifier(Modifier::BOLD),
            )
        })
        .collect()
}

fn battery_icon(fraction: f64) -> &'static str {
    const LEVELS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];
    LEVELS[((fraction.clamp(0.0, 1.0) * 7.0).round() as usize).min(7)]
}

fn level_color(kind: &str) -> Color {
    match kind {
        "P" | "S" => Color::Rgb(255, 110, 150),
        "M" => CPU,
        _ => NPU,
    }
}

fn clock() -> String {
    let now = unsafe { libc::time(std::ptr::null_mut()) };
    let mut parts = unsafe { std::mem::zeroed::<libc::tm>() };
    if unsafe { libc::localtime_r(&now, &mut parts) }.is_null() {
        return String::new();
    }
    format!(
        "{:02}:{:02}:{:02}",
        parts.tm_hour, parts.tm_min, parts.tm_sec
    )
}

// MARK: - Color

/// Green when calm, through yellow and orange, to red when saturated.
fn heat(fraction: f64) -> Color {
    gradient(
        &[
            Color::Rgb(80, 220, 130),
            Color::Rgb(200, 225, 80),
            Color::Rgb(255, 190, 70),
            Color::Rgb(255, 120, 70),
            Color::Rgb(255, 70, 100),
        ],
        fraction,
    )
}

fn gradient(stops: &[Color], fraction: f64) -> Color {
    if stops.is_empty() {
        return TEXT;
    }
    if stops.len() == 1 {
        return stops[0];
    }
    let position = fraction.clamp(0.0, 1.0) * (stops.len() - 1) as f64;
    let index = (position.floor() as usize).min(stops.len() - 2);
    mix(stops[index], stops[index + 1], position - index as f64)
}

fn mix(from: Color, to: Color, amount: f64) -> Color {
    let (Color::Rgb(r1, g1, b1), Color::Rgb(r2, g2, b2)) = (from, to) else {
        return to;
    };
    let blend =
        |a: u8, b: u8| (a as f64 + (b as f64 - a as f64) * amount.clamp(0.0, 1.0)).round() as u8;
    Color::Rgb(blend(r1, r2), blend(g1, g2), blend(b1, b2))
}

fn dim(color: Color, amount: f64) -> Color {
    mix(Color::Rgb(15, 16, 22), color, amount)
}

trait Bright {
    fn max_bright(self) -> Color;
}

impl Bright for Color {
    /// Lifts dark heat colors so badge text stays readable on the border.
    fn max_bright(self) -> Color {
        mix(self, Color::Rgb(255, 255, 255), 0.15)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Renders one real frame off screen. Run with `cargo test render_preview -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn render_preview() {
        let mut sampler = crate::sample::Sampler::new();
        let mut app = App::new();
        let mut reading = sampler.sample(true);
        for _ in 0..12 {
            std::thread::sleep(std::time::Duration::from_millis(150));
            reading = sampler.sample(true);
            app.history.push(&reading, 0.15);
        }
        let width = std::env::var("GLANCE_COLS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(160);
        let height = std::env::var("GLANCE_ROWS")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(48);
        app.anim.tick();
        app.anim.settle();
        if std::env::var("GLANCE_MODE").as_deref() == Ok("about") {
            app.mode = Mode::About;
        }
        if std::env::var("GLANCE_DEMO").is_ok() {
            reading.host.host_name = "macbook-pro".into();
            let home = std::env::var("HOME").unwrap_or_default();
            reading.processes.retain(|process| {
                ["com.apple.", "/System/", "/usr/", "/sbin/"]
                    .iter()
                    .any(|prefix| process.identifier.starts_with(prefix))
            });
            for process in &mut reading.processes {
                process.identifier = process.identifier.replace(&home, "~");
            }
        }
        if std::env::var("GLANCE_MODE").as_deref() == Ok("help") {
            app.mode = Mode::Help;
        }
        if std::env::var("GLANCE_MODE").as_deref() == Ok("confirm") {
            app.mode = Mode::Confirm {
                pid: 4242,
                name: "Safari".into(),
                force: false,
            };
        }
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut app, &reading))
            .unwrap();
        let buffer = terminal.backend().buffer();
        if let Ok(path) = std::env::var("GLANCE_SVG") {
            std::fs::write(path, svg(buffer, width, height)).unwrap();
        }
        if let Ok(path) = std::env::var("GLANCE_HTML") {
            std::fs::write(path, html(buffer, width, height)).unwrap();
        }
        for y in 0..height {
            let line: String = (0..width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect();
            println!("{line}");
        }
    }

    /// Every glyph sits at its own cell position, so fallback fonts cannot drift the grid.
    fn svg(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        const CELL_W: f64 = 8.4;
        const CELL_H: f64 = 17.0;
        const PAD: f64 = 14.0;
        let css = |color: Color| match color {
            Color::Rgb(r, g, b) => Some(format!("#{r:02x}{g:02x}{b:02x}")),
            Color::Black => Some("#000000".into()),
            _ => None,
        };
        let escape = |symbol: &str| match symbol {
            "<" => "&lt;".to_string(),
            ">" => "&gt;".to_string(),
            "&" => "&amp;".to_string(),
            other => other.to_string(),
        };
        let total_w = width as f64 * CELL_W + PAD * 2.0;
        let total_h = height as f64 * CELL_H + PAD * 2.0 + 28.0;
        let mut out = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{total_w}\" height=\"{total_h}\" viewBox=\"0 0 {total_w} {total_h}\">\
<rect width=\"100%\" height=\"100%\" rx=\"10\" fill=\"#0f1016\"/>\
<circle cx=\"22\" cy=\"16\" r=\"5\" fill=\"#ff5f57\"/><circle cx=\"38\" cy=\"16\" r=\"5\" fill=\"#febc2e\"/><circle cx=\"54\" cy=\"16\" r=\"5\" fill=\"#28c840\"/>\
<g transform=\"translate({PAD},{})\" font-family=\"SF Mono, Menlo, Monaco, Consolas, DejaVu Sans Mono, monospace\" font-size=\"13.5\">",
            PAD + 18.0
        );
        for y in 0..height {
            let top = y as f64 * CELL_H;
            let mut x = 0;
            while x < width {
                let bg = buffer[(x, y)].bg;
                let start = x;
                while x < width && buffer[(x, y)].bg == bg {
                    x += 1;
                }
                if let Some(fill) = css(bg) {
                    out.push_str(&format!(
                        "<rect x=\"{:.1}\" y=\"{top:.1}\" width=\"{:.1}\" height=\"{CELL_H}\" fill=\"{fill}\"/>",
                        start as f64 * CELL_W,
                        (x - start) as f64 * CELL_W + 0.4
                    ));
                }
            }
            let mut x = 0;
            while x < width {
                let cell = &buffer[(x, y)];
                let style = (cell.fg, cell.modifier.contains(Modifier::BOLD));
                let mut positions = Vec::new();
                let mut text = String::new();
                while x < width {
                    let cell = &buffer[(x, y)];
                    if (cell.fg, cell.modifier.contains(Modifier::BOLD)) != style {
                        break;
                    }
                    let symbol = cell.symbol();
                    if !symbol.trim().is_empty() {
                        positions.push(format!("{:.1}", x as f64 * CELL_W));
                        text.push_str(&escape(symbol));
                    }
                    x += 1;
                }
                if text.is_empty() {
                    continue;
                }
                let fill = css(style.0).unwrap_or_else(|| "#e1e4f0".into());
                let weight = if style.1 { " font-weight=\"bold\"" } else { "" };
                out.push_str(&format!(
                    "<text x=\"{}\" y=\"{:.1}\" fill=\"{fill}\"{weight} xml:space=\"preserve\">{text}</text>",
                    positions.join(" "),
                    top + CELL_H * 0.76
                ));
            }
        }
        out.push_str("</g></svg>\n");
        out
    }

    fn html(buffer: &ratatui::buffer::Buffer, width: u16, height: u16) -> String {
        let css = |color: Color, fallback: &str| match color {
            Color::Rgb(r, g, b) => format!("rgb({r},{g},{b})"),
            Color::Black => "#000".into(),
            _ => fallback.into(),
        };
        let mut out = String::from(
            "<html><head><meta charset=\"utf-8\"></head><body style=\"margin:0;background:#0f1016\"><pre style=\"font:13px/1.15 Menlo,'SF Mono',monospace;margin:12px;color:#e1e4f0\">",
        );
        for y in 0..height {
            for x in 0..width {
                let cell = &buffer[(x, y)];
                let mut style = format!("color:{}", css(cell.fg, "inherit"));
                if cell.bg != Color::Reset {
                    style.push_str(&format!(";background:{}", css(cell.bg, "transparent")));
                }
                if cell.modifier.contains(Modifier::BOLD) {
                    style.push_str(";font-weight:bold");
                }
                let symbol = match cell.symbol() {
                    "<" => "&lt;",
                    ">" => "&gt;",
                    "&" => "&amp;",
                    other => other,
                };
                out.push_str(&format!("<span style=\"{style}\">{symbol}</span>"));
            }
            out.push('\n');
        }
        out.push_str("</pre></body></html>");
        out
    }
}
