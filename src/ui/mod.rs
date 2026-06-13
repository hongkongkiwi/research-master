//! Modern CLI UI utilities for beautiful terminal output.
//!
//! This module provides colored output, progress indicators, icons,
//! and styled formatting for a polished command-line experience.

use owo_colors::OwoColorize;
use std::io::IsTerminal;
use std::time::Duration;

/// Get the current terminal width.
pub fn terminal_width() -> usize {
    terminal_size::terminal_size()
        .map(|(w, _)| w.0 as usize)
        .unwrap_or(100)
}

/// Check if stdout is a terminal.
pub fn is_terminal() -> bool {
    std::io::stdout().is_terminal()
}

/// Source icons for different research sources.
pub fn source_icon(source: &str) -> &'static str {
    match source.to_lowercase().as_str() {
        "arxiv" => "📝",
        "pubmed" => "🏥",
        "biorxiv" => "🧬",
        "semantic" => "🧠",
        "semantic scholar" => "🧠",
        "openalex" => "🔗",
        "crossref" => "🔗",
        "doi" => "🔗",
        "iacr" => "🔐",
        "pmc" => "📚",
        "pubmed central" => "📚",
        "hal" => "📖",
        "dblp" => "📋",
        "ssrn" => "📊",
        "dimensions" => "📐",
        "ieee" => "⚡",
        "ieee xplore" => "⚡",
        "europe pmc" => "🌍",
        "core" => "💎",
        "zenodo" => "🪐",
        "unpaywall" => "🔓",
        "mdpi" => "📗",
        "jstor" => "📕",
        "scispace" => "🚀",
        "acm" => "💻",
        "connected papers" => "🕸️",
        "doaj" => "📓",
        "worldwidescience" => "🌐",
        "osf" => "☁️",
        "base" => "🔍",
        "springer" => "📚",
        "google scholar" => "🔎",
        _ => "📄",
    }
}

/// Status icons for different operations.
pub fn status_icon(status: Status) -> &'static str {
    match status {
        Status::Success => "✓",
        Status::Error => "✗",
        Status::Warning => "⚠",
        Status::Info => "ℹ",
        Status::Pending => "○",
        Status::Loading => "◐",
        Status::Download => "↓",
        Status::Upload => "↑",
        Status::Search => "🔍",
    }
}

/// Status types for colored output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Success,
    Error,
    Warning,
    Info,
    Pending,
    Loading,
    Download,
    Upload,
    Search,
}

/// Print a styled status message.
#[macro_export]
macro_rules! print_status {
    ($status:expr, $msg:expr) => {
        use $crate::ui::{status_icon, Status};
        let icon = status_icon($status);
        match $status {
            Status::Success => println!("{} {}", icon.green().bold(), $msg),
            Status::Error => println!("{} {}", icon.red().bold(), $msg),
            Status::Warning => println!("{} {}", icon.yellow().bold(), $msg),
            Status::Info => println!("{} {}", icon.cyan().bold(), $msg),
            Status::Pending => println!("{} {}", icon.white().dimmed(), $msg),
            Status::Loading => println!("{} {}", icon.cyan(), $msg),
            Status::Download => println!("{} {}", icon.magenta(), $msg),
            Status::Upload => println!("{} {}", icon.blue(), $msg),
            Status::Search => println!("{} {}", icon.yellow(), $msg),
        }
    };
}

/// Print a styled result message (success/error).
#[macro_export]
macro_rules! print_result {
    ($success:expr, $success_msg:expr, $error_msg:expr) => {
        if $success {
            print_status!(Status::Success, $success_msg);
        } else {
            print_status!(Status::Error, $error_msg);
        }
    };
}

/// Welcome banner for the application.
pub fn print_banner() {
    let version = env!("CARGO_PKG_VERSION");
    let sources_count = 28;

    println!();
    println!("╔═══════════════════════════════════════════════════════════════════════╗");
    println!(
        "║                       🔬 Research Master v{}                          ║",
        version
    );
    println!("║                                                                       ║");
    println!(
        "║   Search & download academic papers from {}+ research sources         ║",
        sources_count
    );
    println!("║                                                                       ║");
    println!("║   Examples:                                                           ║");
    println!("║     research-master search \"transformer attention\"                    ║");
    println!("║     research-master download 2310.12345 --source arxiv               ║");
    println!("║     research-master mcp                                                ║");
    println!("╚═══════════════════════════════════════════════════════════════════════╝");
    println!();
}

/// Print a section header.
pub fn print_section(title: &str) {
    println!();
    println!("{}", format!("━━━ {} ━━━", title).bold().cyan());
}

/// Print a paper in a formatted box.
pub fn print_paper_box(paper: &super::Paper) {
    use owo_colors::OwoColorize;

    let icon = source_icon(&paper.source.to_string());
    let year = paper
        .published_date
        .as_ref()
        .map(|d| d.chars().take(4).collect::<String>())
        .unwrap_or_else(|| "????".to_string());

    println!();
    println!("┌─────────────────────────────────────────────────────────────────────┐");
    println!(
        "│ {} {}                                                    │",
        icon.cyan(),
        paper.title.blue().bold()
    );
    println!("├─────────────────────────────────────────────────────────────────────┤");
    println!(
        "│  Authors: {}                                      │",
        truncate_with_ellipsis(&paper.authors, 60)
    );
    println!(
        "│  Source:  {} {} ({})                                     │",
        icon,
        paper.source.to_string().green(),
        year.yellow()
    );
    if let Some(citations) = paper.citations {
        println!(
            "│  Citations: {}                                                   │",
            citations.to_string().yellow()
        );
    }
    if let Some(doi) = &paper.doi {
        println!(
            "│  DOI: {}                                          │",
            truncate_with_ellipsis(doi, 50)
        );
    }
    println!("└─────────────────────────────────────────────────────────────────────┘");
}

/// Print search results header.
pub fn print_search_header(query: &str, count: usize, duration: Duration) {
    println!();
    println!(
        "{} Search results for: \"{}\"",
        status_icon(Status::Search).yellow().bold(),
        query.cyan().bold()
    );
    println!(
        "{} Found {} papers in {:.2}s",
        "─".repeat(30).dimmed(),
        count.to_string().green().bold(),
        duration.as_secs_f64().white()
    );
    println!();
}

/// Print download progress.
pub fn print_download_progress(paper_id: &str, source: &str, progress: u64, total: u64) {
    let percentage = total
        .checked_div(100)
        .map_or(0.0, |_| (progress * 100 / total) as f64);

    print!(
        "\r{} Downloading {} from {}: {:.1}%",
        status_icon(Status::Download).magenta(),
        paper_id.yellow(),
        source.green(),
        percentage
    );

    if progress >= total {
        println!(); // New line on completion
    }
}

/// Print a divider line.
pub fn print_divider() {
    println!("{}", "─".repeat(80).dimmed());
}

/// Format a number with commas.
pub fn format_number(n: usize) -> String {
    n.to_string()
        .chars()
        .rev()
        .collect::<Vec<_>>()
        .chunks(3)
        .map(|c| c.iter().collect::<String>())
        .collect::<Vec<_>>()
        .join(",")
        .chars()
        .rev()
        .collect()
}

/// Truncate text to fit within the specified width using unicode-aware truncation.
pub fn truncate_with_ellipsis(text: &str, max_width: usize) -> String {
    if max_width == 0 || max_width <= 3 {
        return "...".to_string();
    }

    // Use unicode-width to properly handle wide characters
    let char_widths: Vec<(char, usize)> = text
        .chars()
        .map(|c| (c, unicode_width::UnicodeWidthChar::width(c).unwrap_or(1)))
        .collect();

    let total_width: usize = char_widths.iter().map(|(_, w)| *w).sum();

    if total_width <= max_width {
        return text.to_string();
    }

    // Find the longest prefix that fits
    let mut current_width = 0;
    let mut end_idx = 0;

    for (i, (_, w)) in char_widths.iter().enumerate() {
        if current_width + w > max_width.saturating_sub(3) {
            break;
        }
        current_width += w;
        end_idx = i + 1;
    }

    if end_idx == 0 {
        return "...".to_string();
    }

    let truncated: String = char_widths[..end_idx].iter().map(|(c, _)| *c).collect();
    format!("{}...", truncated)
}

/// Get a human-readable file size.
pub fn format_file_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Multi-source search spinner with animated progress
pub struct MultiSourceSpinner {
    spinner: indicatif::ProgressBar,
    targets: Vec<String>,
    completed: usize,
}

impl MultiSourceSpinner {
    /// Create a new multi-source spinner for tracking parallel searches
    pub fn new(sources: &[&str]) -> Self {
        let targets = sources.iter().map(|s| s.to_string()).collect();
        let count = sources.len();

        let pb = indicatif::ProgressBar::new(count as u64);
        pb.set_style(
            indicatif::ProgressStyle::with_template(
                "{msg}\n{spinner:.cyan} {wide_bar:.cyan/blue} {pos}/{len}",
            )
            .expect("valid progress template")
            .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
            .progress_chars("█   "),
        );

        let mut msg = "🔬 Searching sources".to_string();
        if !sources.is_empty() {
            msg.push_str(&format!(" ({})", sources.join(", ")));
        }
        pb.set_message(msg);

        Self {
            spinner: pb,
            targets,
            completed: 0,
        }
    }

    /// Mark a source as completed
    pub fn complete(&mut self, _source: &str) {
        self.completed += 1;
        self.spinner.inc(1);

        // Update message with completed source
        let completed: Vec<String> = self.targets[..self.completed]
            .iter()
            .map(|s| format!("✓{}", s))
            .collect();
        let pending: Vec<String> = self.targets[self.completed..]
            .iter()
            .map(|s| format!("○{}", s))
            .collect();

        let mut status_parts = completed;
        status_parts.extend(pending);

        let msg = format!("🔬 Searching [{}]", status_parts.join(" "));
        self.spinner.set_message(msg);
    }

    /// Finish with success
    pub fn finish_with_success(&self, total_results: usize) {
        self.spinner.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("valid progress template")
                .tick_chars("✓"),
        );
        self.spinner
            .finish_with_message(format!("✓ Found {} papers", total_results));
    }

    /// Finish with error
    pub fn finish_with_error(&self, msg: &str) {
        self.spinner.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.red} {msg}")
                .expect("valid progress template")
                .tick_chars("✗"),
        );
        self.spinner.finish_with_message(format!("✗ {}", msg));
    }
}

/// Scientific loading spinner with themed animation
pub struct ScientificSpinner {
    pb: indicatif::ProgressBar,
}

impl ScientificSpinner {
    /// Create a new scientific-themed spinner
    pub fn new(msg: &str) -> Self {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner} {msg}")
                .expect("valid progress template")
                .tick_chars("🔬 ⚗️ 🧪 🧫 🔭 📡 🧬 ⚛️ "),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(150));

        Self { pb }
    }

    /// Set a new message
    pub fn set_message(&self, msg: &str) {
        self.pb.set_message(msg.to_string());
    }

    /// Update to a sub-operation
    pub fn update(&self, current: usize, total: usize) {
        let percent = total
            .checked_div(1)
            .map_or(0, |_| (current * 100 / total).min(100));
        let msg = format!("({}/{}) {}%", current, total, percent);
        self.pb.set_message(msg);
    }

    /// Finish with success
    pub fn finish_with_success(&self, msg: &str) {
        self.pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("valid progress template")
                .tick_chars("✓"),
        );
        self.pb.finish_with_message(msg.to_string());
    }

    /// Finish with error
    pub fn finish_with_error(&self, msg: &str) {
        self.pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.red} {msg}")
                .expect("valid progress template")
                .tick_chars("✗"),
        );
        self.pb.finish_with_message(msg.to_string());
    }

    /// Finish the spinner
    pub fn finish(&self) {
        self.pb.finish();
    }
}

/// Print a loading spinner with message.
pub struct Spinner {
    pb: indicatif::ProgressBar,
}

impl Spinner {
    /// Create a new spinner with the given message.
    pub fn new(msg: &str) -> Self {
        let pb = indicatif::ProgressBar::new_spinner();
        pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.cyan} {msg}")
                .expect("valid progress template")
                .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ "),
        );
        pb.set_message(msg.to_string());
        pb.enable_steady_tick(Duration::from_millis(100));

        Self { pb }
    }

    /// Set the message.
    pub fn set_message(&self, msg: &str) {
        self.pb.set_message(msg.to_string());
    }

    /// Finish with success message.
    pub fn finish_with_success(&self, msg: &str) {
        self.pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.green} {msg}")
                .expect("valid progress template")
                .tick_chars("✓ ✗ "),
        );
        self.pb.finish_with_message(msg.to_string());
    }

    /// Finish with error message.
    pub fn finish_with_error(&self, msg: &str) {
        self.pb.set_style(
            indicatif::ProgressStyle::with_template("{spinner:.red} {msg}")
                .expect("valid progress template")
                .tick_chars("✓ ✗ "),
        );
        self.pb.finish_with_message(msg.to_string());
    }

    /// Update to indeterminate progress.
    pub fn set_length(&self, len: u64) {
        self.pb.set_length(len);
    }

    /// Increment progress.
    pub fn inc(&self, delta: u64) {
        self.pb.inc(delta);
    }

    /// Set progress.
    pub fn set_position(&self, pos: u64) {
        self.pb.set_position(pos);
    }

    /// Finish the spinner.
    pub fn finish(&self) {
        self.pb.finish();
    }
}

/// Create a progress bar for downloads.
pub fn create_progress_bar(len: u64, msg: &str) -> Spinner {
    let pb = indicatif::ProgressBar::new(len);
    pb.set_style(
        indicatif::ProgressStyle::with_template(
            "{msg}: {bar:40.cyan/blue} {pos}/{len} ({percent}%)",
        )
        .expect("valid progress template")
        .progress_chars("█▓▒░ "),
    );
    pb.set_message(msg.to_string());

    Spinner { pb }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_icon() {
        assert_eq!(source_icon("arxiv"), "📝");
        assert_eq!(source_icon("pubmed"), "🏥");
        assert_eq!(source_icon("semantic scholar"), "🧠");
        assert_eq!(source_icon("unknown"), "📄");
    }

    #[test]
    fn test_status_icon() {
        assert_eq!(status_icon(Status::Success), "✓");
        assert_eq!(status_icon(Status::Error), "✗");
        assert_eq!(status_icon(Status::Search), "🔍");
    }

    #[test]
    fn test_truncate_with_ellipsis() {
        assert_eq!(truncate_with_ellipsis("Hello", 10), "Hello");
        assert_eq!(truncate_with_ellipsis("Hello World", 8), "Hello...");
        assert_eq!(truncate_with_ellipsis("Hi", 10), "Hi");
        assert_eq!(truncate_with_ellipsis("", 10), "");
        assert_eq!(truncate_with_ellipsis("Hello", 3), "...");
    }

    #[test]
    fn test_format_number() {
        assert_eq!(format_number(1000), "1,000");
        assert_eq!(format_number(1000000), "1,000,000");
        assert_eq!(format_number(123), "123");
    }

    #[test]
    fn test_format_file_size() {
        assert_eq!(format_file_size(500), "500 B");
        assert_eq!(format_file_size(1024), "1.00 KB");
        assert_eq!(format_file_size(1048576), "1.00 MB");
    }
}
