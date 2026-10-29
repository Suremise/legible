//! Legible's platform-free core: rendering Markdown to safe HTML, the clipboard's HTML, saving,
//! the strings and the embedded web assets. Nothing here calls Windows: that's `legible-win`.
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
