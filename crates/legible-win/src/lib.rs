//! Legible's Windows layer: where its files live and its logs, the document windows and their
//! WebView2, and later the clipboard and file associations. Platform-free code belongs in
//! `legible-core`.

#[cfg(windows)]
pub mod about;
#[cfg(windows)]
pub mod actions;
#[cfg(windows)]
pub mod associations;
#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
pub mod confine;
#[cfg(windows)]
pub mod consent;
#[cfg(windows)]
pub mod disk;
#[cfg(windows)]
pub mod editor;
#[cfg(windows)]
pub mod export;
#[cfg(windows)]
pub mod instance;
#[cfg(windows)]
pub mod jumplist;
#[cfg(windows)]
pub mod links;
#[cfg(windows)]
pub mod logging;
#[cfg(windows)]
pub mod menu;
#[cfg(windows)]
pub mod page;
#[cfg(windows)]
pub mod paths;
#[cfg(windows)]
pub mod placement;
#[cfg(windows)]
pub mod places;
#[cfg(windows)]
pub mod quiet;
#[cfg(windows)]
pub mod reason;
#[cfg(windows)]
pub mod recent;
#[cfg(windows)]
pub mod recovery;
#[cfg(windows)]
pub mod reload;
#[cfg(windows)]
pub mod saving;
#[cfg(windows)]
pub mod session;
#[cfg(windows)]
pub mod settings;
#[cfg(windows)]
pub mod shell;
#[cfg(windows)]
pub mod status;
#[cfg(windows)]
pub mod update;
#[cfg(windows)]
pub mod watch;
#[cfg(windows)]
pub mod win32;
