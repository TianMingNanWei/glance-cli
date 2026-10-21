pub const NAME: &str = "Glance CLI";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
pub const AUTHOR: &str = "SunJiaxuan";
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const LICENSE: &str = env!("CARGO_PKG_LICENSE");

pub const BANNER: [&str; 6] = [
    " ██████╗ ██╗      █████╗ ███╗   ██╗ ██████╗███████╗",
    "██╔════╝ ██║     ██╔══██╗████╗  ██║██╔════╝██╔════╝",
    "██║  ███╗██║     ███████║██╔██╗ ██║██║     █████╗  ",
    "██║   ██║██║     ██╔══██║██║╚██╗██║██║     ██╔══╝  ",
    "╚██████╔╝███████╗██║  ██║██║ ╚████║╚██████╗███████╗",
    " ╚═════╝ ╚══════╝╚═╝  ╚═╝╚═╝  ╚═══╝ ╚═════╝╚══════╝",
];

/// Label and value pairs shown by `glance about` and the About pop-up.
pub fn details() -> Vec<(&'static str, String)> {
    vec![
        ("Version", VERSION.to_string()),
        ("Author", AUTHOR.to_string()),
        ("Source", REPOSITORY.to_string()),
        ("License", LICENSE.to_string()),
        ("Built with", "Rust · ratatui · crossterm".to_string()),
        (
            "Data",
            "host statistics, IOReport, IOKit, HID sensors — no root".to_string(),
        ),
        (
            "Platform",
            format!("macOS 14+ · {}", std::env::consts::ARCH),
        ),
    ]
}
