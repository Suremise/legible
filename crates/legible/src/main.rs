//! Legible: a Markdown reader for Windows.
//!
//! `legible [FILE]...` opens each file in a window of its own. The first Legible to start opens
//! the windows; any started later (a double-click in Explorer) hands its files to it and exits,
//! so a file that's already open has its window brought forward.

// A windowed app: no console window when started from Explorer.
#![cfg_attr(windows, windows_subsystem = "windows")]

/// The app's name for its folders, logs and single-instance pipe (not translated).
#[cfg(windows)]
const APP: &str = "Legible";

#[cfg(windows)]
fn main() {
    use legible_win::instance::{self, Claim};
    use legible_win::logging::{self, LogConfig};
    use legible_win::paths::AppPaths;
    use legible_win::{shell, win32};

    let lang = win32::ui_language().unwrap_or_else(|| "en".into());
    legible_core::i18n::init(&lang);
    let paths = AppPaths::detect(APP).ok();
    let logs = paths
        .as_ref()
        .filter(|p| p.is_writable())
        .map(|p| p.logs_dir());
    // No log file is no reason not to start.
    let _ = logging::init(&LogConfig::new(logs, "legible"));
    logging::log_panics();
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        mode = paths.as_ref().map(|p| p.mode().as_str()),
        "started"
    );

    // Before the single-instance pipe, so a Legible that can't show anything never holds it.
    match shell::webview2_version() {
        Some(version) => tracing::info!(version, "WebView2"),
        None => {
            tracing::error!("WebView2 isn't installed");
            shell::report_missing_webview2();
            std::process::exit(1);
        }
    }

    let files = files_from(std::env::args_os().skip(1));
    let primary = match instance::pipe_name(APP) {
        Ok(name) => match instance::claim(&name, &files) {
            Claim::HandedOver => {
                tracing::info!(files = files.len(), "handed over to the open Legible");
                return;
            }
            Claim::Primary(primary) => primary,
        },
        Err(error) => {
            tracing::warn!(%error, "no single-instance pipe");
            None
        }
    };

    shell::run(shell::Options {
        paths,
        files,
        primary,
        lang,
    });
}

/// The command line's files, made absolute (the process they're handed to may have another
/// current folder). Arguments starting with `-` are left for options to come.
#[cfg(windows)]
fn files_from(args: impl Iterator<Item = std::ffi::OsString>) -> Vec<std::path::PathBuf> {
    args.filter(|a| !a.to_string_lossy().starts_with('-'))
        .map(std::path::PathBuf::from)
        .map(|p| std::path::absolute(&p).unwrap_or(p))
        .collect()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("{}", legible_core::tr!("app-windows-only"));
    std::process::exit(1);
}
