//! What each kind of link does, as a table: the href as the document wrote it, and the action.
#![cfg(windows)]

use std::fs;
use std::path::{Path, PathBuf};

use legible_win::links::{
    LinkAction, Refused, classify, classify_without_document, dropped_markdown,
};

struct Folder {
    _temp: tempfile::TempDir,
    root: PathBuf,
    document: PathBuf,
}

/// `root\docs\readme.md` (the document), with files beside, below and above it.
fn folder() -> Folder {
    let temp = tempfile::tempdir().unwrap();
    let root = fs::canonicalize(temp.path()).unwrap();
    let docs = root.join("docs");
    fs::create_dir_all(docs.join("sub")).unwrap();
    for file in [
        "docs/readme.md",
        "docs/other.md",
        "docs/sub/deep.MD",
        "docs/space name.md",
        "docs/notes.markdown",
        "docs/report.pdf",
        "parent.md",
    ] {
        fs::write(root.join(file), "x").unwrap();
    }
    Folder {
        _temp: temp,
        document: docs.join("readme.md"),
        root,
    }
}

fn open(path: &Path) -> LinkAction {
    LinkAction::OpenDocument(fs::canonicalize(path).unwrap())
}

#[test]
fn every_kind_of_link() {
    let f = folder();
    let docs = f.root.join("docs");
    let parent = f.root.join("parent.md");
    let plain_parent = parent.display().to_string();
    let plain_parent = plain_parent.trim_start_matches(r"\\?\");
    let parent_url = format!("file:///{}", plain_parent.replace('\\', "/"));
    let browser = |url: &str| LinkAction::Browser(url.to_owned());
    let refuse = LinkAction::Refuse;
    let table: Vec<(&str, LinkAction)> = vec![
        // Within the page.
        ("#intro", LinkAction::InPage),
        ("  #intro", LinkAction::InPage),
        ("", LinkAction::InPage),
        // The web.
        (
            "https://example.com/a?b=1#c",
            browser("https://example.com/a?b=1#c"),
        ),
        ("HTTPS://example.com", browser("HTTPS://example.com")),
        (" https://example.com/ ", browser("https://example.com/")),
        (
            "https://user@example.com/",
            browser("https://user@example.com/"),
        ),
        ("https://例え.jp/", browser("https://例え.jp/")),
        (
            "http://example.com/",
            LinkAction::BrowserAfterAsking("http://example.com/".into()),
        ),
        ("https://", refuse(Refused::BadUrl)),
        ("https:example.com", refuse(Refused::BadUrl)),
        ("https://exa mple.com", refuse(Refused::BadUrl)),
        ("https://example.com/\"onclick", refuse(Refused::BadUrl)),
        ("https://example.com/\u{85}x", refuse(Refused::BadUrl)),
        // Mail.
        (
            "mailto:someone@example.com",
            LinkAction::Mail("mailto:someone@example.com".into()),
        ),
        (
            "MailTo:someone@example.com",
            LinkAction::Mail("MailTo:someone@example.com".into()),
        ),
        // Schemes that are never followed, however they're written.
        ("javascript:alert(1)", refuse(Refused::Scheme)),
        ("JaVaScRiPt:alert(1)", refuse(Refused::Scheme)),
        ("java\nscript:alert(1)", refuse(Refused::Scheme)),
        ("\u{1}javascript:alert(1)", refuse(Refused::Scheme)),
        ("vbscript:msgbox", refuse(Refused::Scheme)),
        (
            "data:text/html,<script>alert(1)</script>",
            refuse(Refused::Scheme),
        ),
        ("blob:https://legible.localhost/x", refuse(Refused::Scheme)),
        ("about:blank", refuse(Refused::Scheme)),
        ("ftp://example.com/a.md", refuse(Refused::Scheme)),
        ("ms-settings:privacy", refuse(Refused::Scheme)),
        ("search-ms:query=x", refuse(Refused::Scheme)),
        ("legible-doc://localhost/a.png", refuse(Refused::Scheme)),
        // Other Markdown files, relative to the document.
        ("other.md", open(&docs.join("other.md"))),
        ("./other.md#section", open(&docs.join("other.md"))),
        ("other.md?plain=1", open(&docs.join("other.md"))),
        ("sub/deep.MD", open(&docs.join("sub/deep.MD"))),
        (r"sub\deep.MD", open(&docs.join("sub/deep.MD"))),
        ("../parent.md", open(&parent)),
        ("space%20name.md", open(&docs.join("space name.md"))),
        ("notes.markdown", open(&docs.join("notes.markdown"))),
        // By their own path.
        (plain_parent, open(&parent)),
        (&parent_url, open(&parent)),
        // Files that aren't there, aren't Markdown, or can't be placed.
        ("missing.md", refuse(Refused::NotFound)),
        ("sub", refuse(Refused::NotMarkdown)),
        ("report.pdf", refuse(Refused::NotMarkdown)),
        ("file:///C:/Windows/win.ini", refuse(Refused::NotMarkdown)),
        (r"C:\Windows\System32\cmd.exe", refuse(Refused::NotMarkdown)),
        ("/docs/other.md", refuse(Refused::Unplaceable)),
        ("//host/share/a.md", refuse(Refused::Unplaceable)),
        ("C:other.md", refuse(Refused::Unplaceable)),
        ("%zz.md", refuse(Refused::Unplaceable)),
        ("file:a.md", refuse(Refused::Unplaceable)),
    ];
    let mut wrong = Vec::new();
    for (href, expected) in &table {
        let actual = classify(href, &f.document);
        if &actual != expected {
            wrong.push(format!("{href:?}: expected {expected:?}, got {actual:?}"));
        }
    }
    assert!(wrong.is_empty(), "\n{}", wrong.join("\n"));
}

#[test]
fn a_markdown_folder_is_not_a_document() {
    let f = folder();
    fs::create_dir(f.root.join("docs").join("folder.md")).unwrap();
    assert_eq!(
        classify("folder.md", &f.document),
        LinkAction::Refuse(Refused::NotFound)
    );
}

#[test]
fn a_drop_opens_its_markdown_files_only() {
    let f = folder();
    let docs = f.root.join("docs");
    fs::create_dir(docs.join("folder.md")).unwrap();
    fs::write(docs.join("notes.md.txt"), "x").unwrap();
    fs::write(docs.join("LOUD.MARKDOWN"), "x").unwrap();
    fs::write(docs.join("old.Mdown"), "x").unwrap();
    let dropped = [
        "report.pdf",
        "readme.md",
        "folder.md",
        "notes.md.txt",
        "sub",
        "LOUD.MARKDOWN",
        "old.Mdown",
        "missing.md",
    ]
    .map(|name| docs.join(name));
    let expected = ["readme.md", "LOUD.MARKDOWN", "old.Mdown", "missing.md"].map(|n| docs.join(n));
    // A missing file is kept: opening it says it couldn't, as a double-click would.
    assert_eq!(dropped_markdown(dropped.to_vec()), expected.to_vec());
    assert!(dropped_markdown(vec![docs.join("report.pdf"), docs.join("sub")]).is_empty());
}

#[test]
fn with_no_document_web_and_mail_links_go_out_and_paths_do_nothing() {
    // As About's release notes in the empty window: no document to place a path by.
    let cases = [
        ("#unreleased", LinkAction::InPage),
        (
            "https://keepachangelog.com/en/1.1.0/",
            LinkAction::Browser("https://keepachangelog.com/en/1.1.0/".into()),
        ),
        (
            "http://example.com/",
            LinkAction::BrowserAfterAsking("http://example.com/".into()),
        ),
        (
            "mailto:someone@example.com",
            LinkAction::Mail("mailto:someone@example.com".into()),
        ),
        ("notes.md", LinkAction::Refuse(Refused::Unplaceable)),
        (r"C:\notes\a.md", LinkAction::Refuse(Refused::Unplaceable)),
        (
            "file:///C:/notes/a.md",
            LinkAction::Refuse(Refused::Unplaceable),
        ),
        ("javascript:alert(1)", LinkAction::Refuse(Refused::Scheme)),
        ("https://", LinkAction::Refuse(Refused::BadUrl)),
    ];
    for (href, action) in cases {
        assert_eq!(classify_without_document(href), action, "{href}");
    }
}
