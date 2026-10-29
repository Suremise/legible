//! The outline: the document's headings, each with the id its `<h1>`…`<h6>` carries, for the
//! side panel. Built while the HTML is written, so the ids match.

use std::fmt;
use std::sync::Mutex;

use comrak::Anchorizer;
use comrak::adapters::{HeadingAdapter, HeadingMeta};
use comrak::nodes::Sourcepos;

use crate::frontmatter::escape;

/// The prefix of every id in a rendered document, GitHub's, so no id the document makes can
/// clash with one Legible's own page uses.
pub const ID_PREFIX: &str = "user-content-";

/// One heading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    /// 1 to 6.
    pub level: u8,
    /// Its text, without formatting.
    pub text: String,
    /// The id of its element in the HTML, prefixed: `user-content-<slug>`.
    pub id: String,
}

/// Writes each heading's opening tag with its id (a GitHub-style slug: lower case, punctuation
/// dropped, spaces to hyphens, a repeat given `-1`, `-2`…) and records it, prefixed as the
/// sanitised HTML has it.
#[derive(Default)]
pub(crate) struct HeadingIds {
    state: Mutex<(Anchorizer, Vec<Heading>)>,
}

impl HeadingIds {
    pub(crate) fn into_outline(self) -> Vec<Heading> {
        self.state.into_inner().unwrap_or_else(|e| e.into_inner()).1
    }
}

impl HeadingAdapter for HeadingIds {
    fn enter(
        &self,
        output: &mut dyn fmt::Write,
        heading: &HeadingMeta,
        sourcepos: Option<Sourcepos>,
    ) -> fmt::Result {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // The bare slug in the HTML: sanitising prefixes every id, as it does every `#fragment`.
        let slug = state.0.anchorize(&heading.content);
        // Its lines, when the renderer was asked for them (comrak passes them only then).
        let lines = sourcepos.map_or(String::new(), |pos| format!(" data-sourcepos=\"{pos}\""));
        write!(
            output,
            "<h{}{lines} id=\"{}\">",
            heading.level,
            escape(&slug)
        )?;
        state.1.push(Heading {
            level: heading.level,
            text: heading.content.clone(),
            id: format!("{ID_PREFIX}{slug}"),
        });
        Ok(())
    }

    fn exit(&self, output: &mut dyn fmt::Write, heading: &HeadingMeta) -> fmt::Result {
        writeln!(output, "</h{}>", heading.level)
    }
}
