//! Front matter: a block of YAML (between `---` lines) or TOML (between `+++` lines) at the very
//! top of a document, shown as its properties rather than as text.
//!
//! Only a block that parses, as a mapping of keys to values, counts: anything else (a document
//! that starts with a horizontal rule, say) is left in the body as Markdown. Values are shown as
//! text, in the language's words (`properties-*`): true and false, lists joined with commas,
//! nested tables as `key: value` pairs.
//!
//! A document is untrusted, so the block is bounded before it's parsed: at most [`MAX_BYTES`],
//! and for YAML at most [`MAX_EVENTS`] parser events nested at most [`MAX_DEPTH`] deep, with no
//! aliases (`*name`). An alias repeats what its anchor holds, so a few hundred bytes of nested
//! aliases can stand for gigabytes; front matter with one is left as Markdown. With no aliases,
//! the properties' text is never longer than the block.

use crate::tr;

/// The largest front matter read as properties.
pub const MAX_BYTES: usize = 64 * 1024;
/// The most YAML parser events (scalars, starts and ends of lists and mappings) in front matter.
pub const MAX_EVENTS: usize = 10_000;
/// The deepest YAML lists and mappings nest in front matter.
pub const MAX_DEPTH: usize = 32;

/// One property: a key and its value, as text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub key: String,
    pub value: String,
}

/// Splits `markdown` into its properties and the body after the front matter. With no front
/// matter (or one that doesn't parse), the properties are empty and the body is all of it.
pub fn split(markdown: &str) -> (Vec<Property>, &str) {
    let text = markdown.strip_prefix('\u{feff}').unwrap_or(markdown);
    for (fence, parse) in [
        ("---", parse_yaml as fn(&str) -> Option<Vec<Property>>),
        ("+++", parse_toml),
    ] {
        if let Some((block, body)) = fenced(text, fence)
            && block.len() <= MAX_BYTES
            && let Some(properties) = parse(block)
        {
            return (properties, body);
        }
    }
    (Vec::new(), markdown)
}

/// The block between a first line of `fence` and the next line of `fence` (YAML may also close
/// with `...`), and the text after it.
fn fenced<'a>(text: &'a str, fence: &str) -> Option<(&'a str, &'a str)> {
    let rest = text.strip_prefix(fence)?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\r', '\n']).trim_end();
        if bare == fence || (fence == "---" && bare == "...") {
            return Some((&rest[..offset], &rest[offset + line.len()..]));
        }
        offset += line.len();
    }
    None
}

fn parse_yaml(block: &str) -> Option<Vec<Property>> {
    use yaml_rust2::{Yaml, YamlLoader};
    if !yaml_is_bounded(block) {
        return None;
    }
    let docs = YamlLoader::load_from_str(block).ok()?;
    let Some(Yaml::Hash(map)) = docs.into_iter().next() else {
        return None;
    };
    Some(
        map.iter()
            .map(|(key, value)| Property {
                key: yaml_text(key),
                value: yaml_text(value),
            })
            .collect(),
    )
}

/// Whether the YAML parses within the limits and has no alias, checked on the parser's events,
/// which come one at a time and never expand anything, before the loader builds a tree.
fn yaml_is_bounded(block: &str) -> bool {
    use yaml_rust2::parser::{Event, Parser};
    let mut parser = Parser::new_from_str(block);
    let mut depth = 0usize;
    for _ in 0..MAX_EVENTS {
        match parser.next_token() {
            Err(_) | Ok((Event::Alias(_), _)) => return false,
            Ok((Event::SequenceStart(..) | Event::MappingStart(..), _)) => {
                depth += 1;
                if depth > MAX_DEPTH {
                    return false;
                }
            }
            Ok((Event::SequenceEnd | Event::MappingEnd, _)) => depth = depth.saturating_sub(1),
            Ok((Event::StreamEnd, _)) => return true,
            Ok(_) => {}
        }
    }
    false
}

fn yaml_text(value: &yaml_rust2::Yaml) -> String {
    use yaml_rust2::Yaml;
    match value {
        Yaml::String(s) | Yaml::Real(s) => s.clone(),
        Yaml::Integer(n) => n.to_string(),
        Yaml::Boolean(b) => boolean(*b),
        Yaml::Array(items) => list(items.iter().map(yaml_text)),
        Yaml::Hash(map) => list(map.iter().map(|(k, v)| entry(&yaml_text(k), &yaml_text(v)))),
        Yaml::Null | Yaml::BadValue | Yaml::Alias(_) => String::new(),
    }
}

fn parse_toml(block: &str) -> Option<Vec<Property>> {
    let table: toml::Table = block.parse().ok()?;
    Some(
        table
            .iter()
            .map(|(key, value)| Property {
                key: key.clone(),
                value: toml_text(value),
            })
            .collect(),
    )
}

fn toml_text(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Boolean(b) => boolean(*b),
        toml::Value::Array(items) => list(items.iter().map(toml_text)),
        toml::Value::Table(table) => list(table.iter().map(|(k, v)| entry(k, &toml_text(v)))),
        other => other.to_string(),
    }
}

fn boolean(value: bool) -> String {
    if value {
        tr!("properties-true")
    } else {
        tr!("properties-false")
    }
}

fn list(items: impl Iterator<Item = String>) -> String {
    items
        .collect::<Vec<_>>()
        .join(&tr!("properties-list-separator"))
}

fn entry(key: &str, value: &str) -> String {
    tr!(
        "properties-entry",
        key = key.to_owned(),
        value = value.to_owned()
    )
}

/// The most properties shown open; more start collapsed.
pub const OPEN_PROPERTIES: usize = 6;

/// The properties as a collapsible table, `<details class="properties">`, open unless there are
/// more than [`OPEN_PROPERTIES`], or nothing when there are none. Everything in it is escaped:
/// it's added after sanitising.
pub fn to_html(properties: &[Property]) -> String {
    if properties.is_empty() {
        return String::new();
    }
    let open = if properties.len() > OPEN_PROPERTIES {
        ""
    } else {
        " open"
    };
    let mut html = format!(
        "<details class=\"properties\"{open}><summary>{}</summary>\n<table>\n<tbody>\n",
        escape(&tr!("properties-title"))
    );
    for p in properties {
        html.push_str(&format!(
            "<tr><th scope=\"row\">{}</th><td>{}</td></tr>\n",
            escape(&p.key),
            escape(&p.value)
        ));
    }
    html.push_str("</tbody>\n</table>\n</details>\n");
    html
}

/// Escapes text for HTML content and double-quoted attribute values.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(key: &str, value: &str) -> Property {
        Property {
            key: key.into(),
            value: value.into(),
        }
    }

    #[test]
    fn yaml_front_matter_becomes_properties_in_order() {
        let (props, body) =
            split("---\ntitle: Report\ntags: [a, b]\ndraft: true\nn: 3\n---\n# Hi\n");
        assert_eq!(
            props,
            [
                p("title", "Report"),
                p("tags", "a, b"),
                p("draft", "true"),
                p("n", "3")
            ]
        );
        assert_eq!(body, "# Hi\n");
    }

    #[test]
    fn toml_front_matter_becomes_properties_in_order() {
        let (props, body) =
            split("+++\nzeta = \"last\"\nalpha = [1, 2]\n[owner]\nname = \"Ann\"\n+++\nBody");
        assert_eq!(
            props,
            [
                p("zeta", "last"),
                p("alpha", "1, 2"),
                p("owner", "name: Ann")
            ]
        );
        assert_eq!(body, "Body");
    }

    #[test]
    fn crlf_a_bom_and_a_yaml_end_marker() {
        let (props, body) = split("\u{feff}---\r\na: 1\r\n...\r\nText");
        assert_eq!(props, [p("a", "1")]);
        assert_eq!(body, "Text");
    }

    #[test]
    fn what_isnt_a_mapping_stays_markdown() {
        for text in [
            "---\nJust a paragraph\n---\nmore",
            "---\n- a\n- b\n---\n",
            "---\na: [unclosed\n---\n",
            "---\nno end\n",
            "Text\n---\na: 1\n---\n",
            "+++\nnot = toml = here\n+++\n",
        ] {
            let (props, body) = split(text);
            assert!(props.is_empty(), "{text:?}");
            assert_eq!(body, text);
        }
    }

    /// Nested aliases: each level repeats the one before four times, so ten levels stand for a
    /// million items in a few hundred bytes. Left as Markdown, at once.
    #[test]
    fn an_alias_bomb_is_left_as_markdown() {
        let mut yaml = String::from(
            "---
a0: &a0 [x, x, x, x]
",
        );
        for n in 1..10 {
            let p = n - 1;
            yaml.push_str(&format!(
                "a{n}: &a{n} [*a{p}, *a{p}, *a{p}, *a{p}]
"
            ));
        }
        yaml.push_str(
            "---
Body
",
        );
        assert!(yaml.len() < 400);
        let (props, body) = split(&yaml);
        assert!(props.is_empty());
        assert_eq!(body, yaml);
        // A single alias is enough to leave it alone.
        let (props, _) = split(
            "---
a: &x 1
b: *x
---
",
        );
        assert!(props.is_empty());
        // An anchor alone is fine.
        assert_eq!(
            split(
                "---
a: &x 1
---
"
            )
            .0,
            [p("a", "1")]
        );
    }

    #[test]
    fn oversized_or_deep_front_matter_is_left_as_markdown() {
        let big = format!(
            "---
a: \"{}\"
---
",
            "x".repeat(MAX_BYTES)
        );
        assert!(split(&big).0.is_empty());
        let deep = format!(
            "---
a: {}1{}
---
",
            "[".repeat(MAX_DEPTH + 1),
            "]".repeat(MAX_DEPTH + 1)
        );
        assert!(split(&deep).0.is_empty());
        let fine = format!(
            "---
a: {}1{}
---
",
            "[".repeat(MAX_DEPTH - 2),
            "]".repeat(MAX_DEPTH - 2)
        );
        assert_eq!(split(&fine).0, [p("a", "1")]);
        let many = format!(
            "---
a: [{}]
---
",
            vec!["1"; MAX_EVENTS].join(", ")
        );
        assert!(split(&many).0.is_empty());
        // TOML nested past its parser's own limit fails to parse, without overflowing the stack.
        let toml = format!(
            "+++
a = {}1{}
+++
",
            "[".repeat(5000),
            "]".repeat(5000)
        );
        assert!(split(&toml).0.is_empty());
    }

    #[test]
    fn the_table_is_escaped() {
        let html = to_html(&[p("<b>", "\"x\" & 'y'")]);
        assert!(
            html.starts_with("<details class=\"properties\" open><summary>Properties</summary>")
        );
        assert!(
            html.contains(
                "<th scope=\"row\">&lt;b&gt;</th><td>&quot;x&quot; &amp; &#39;y&#39;</td>"
            )
        );
        assert_eq!(to_html(&[]), "");
    }

    #[test]
    fn a_long_table_starts_collapsed() {
        let six: Vec<_> = (0..6).map(|i| p(&format!("k{i}"), "v")).collect();
        assert!(to_html(&six).starts_with("<details class=\"properties\" open>"));
        let seven: Vec<_> = (0..7).map(|i| p(&format!("k{i}"), "v")).collect();
        assert!(to_html(&seven).starts_with("<details class=\"properties\"><summary>"));
    }
}
