//! Legible's platform-free core: rendering Markdown to safe HTML, the clipboard's HTML, saving,
//! the strings, the embedded web assets and the update check's logic. Nothing here calls Windows: that's `legible-win`.
//!
//! **Rendering** is [`render::render`]: Markdown in, a [`render::Rendered`] out (the sanitised
//! HTML, the outline, the front matter's properties, and whether it has remote images or Mermaid
//! diagrams).

pub mod assets;
pub mod clipboard_html;
pub mod frontmatter;
pub mod i18n;
pub mod outline;
pub mod render;
mod sanitize;
pub mod save;
pub mod unsaved;
pub mod update;
