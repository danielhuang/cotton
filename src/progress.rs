use std::time::Duration;

use indicatif::{ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use owo_colors::OwoColorize;

use crate::ARGS;

pub static PROGRESS_BAR: Lazy<ProgressBar> = Lazy::new(|| {
    let pb = ProgressBar::new(0).with_style(
        ProgressStyle::with_template("{spinner:.blue} {wide_msg} +{pos:.green} ~{len:.magenta}")
            .unwrap()
            .progress_chars("#>-")
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏"),
    );
    pb.enable_steady_tick(Duration::from_millis(200));
    pb
});

pub fn log_verbose(text: &str) {
    if ARGS.verbose {
        PROGRESS_BAR.println(format!("{} {}", " VERBOSE ".on_white(), text));
    }
}

pub fn log_warning(text: &str) {
    PROGRESS_BAR.println(format!("{} {}", " WARNING ".on_yellow(), text));
}

pub fn log_progress(text: &str) {
    PROGRESS_BAR.set_message(text.to_string());
    log_verbose(text);
}
