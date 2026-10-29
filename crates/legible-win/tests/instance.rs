//! The single-instance broker: a second launch, as a process of its own, hands its files to
//! the first and exits. This test exe is the first; it runs itself again as the second.
#![cfg(windows)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::mpsc;
use std::time::Duration;

use legible_win::instance::{self, Claim};

const CHILD: &str = "LEGIBLE_INSTANCE_TEST_CHILD";
const CHILD_FILES: &str = "LEGIBLE_INSTANCE_TEST_FILES";

/// A pipe of this test's own, so it never meets a real Legible.
fn test_pipe(test: &str) -> String {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!(
        r"\\.\pipe\legible-test-{test}-{}-{unique}",
        std::process::id()
    )
}

/// Run only as the second launch: claims the pipe with its files and says what it became.
#[test]
fn child() {
    let Some(name) = std::env::var_os(CHILD) else {
        return;
    };
    let files: Vec<PathBuf> = std::env::var_os(CHILD_FILES)
        .map(|f| std::env::split_paths(&f).collect())
        .unwrap_or_default();
    match instance::claim(&name.to_string_lossy(), &files) {
        Claim::HandedOver => println!("CLAIM=handed-over"),
        Claim::Primary(Some(_)) => println!("CLAIM=primary"),
        Claim::Primary(None) => println!("CLAIM=primary-without-pipe"),
    }
}

/// Runs this test exe again as a second launch with `files`, and returns what it became.
fn second_launch(name: &str, files: &[PathBuf]) -> String {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "child", "--nocapture", "--test-threads=1"])
        .env(CHILD, name)
        // A test's launch passes no right to the foreground on.
        .env(legible_win::quiet::VAR, "1");
    if !files.is_empty() {
        command.env(CHILD_FILES, std::env::join_paths(files).unwrap());
    }
    let output = command.output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    // libtest prints "test child ... " before the test's own output, on the same line.
    stdout
        .lines()
        .find_map(|l| l.split_once("CLAIM=").map(|(_, claim)| claim.trim()))
        .unwrap_or_else(|| panic!("the second launch said nothing: {stdout}"))
        .to_owned()
}

#[test]
fn a_second_launch_hands_its_files_over() {
    let name = test_pipe("handover");
    let Claim::Primary(Some(primary)) = instance::claim(&name, &[]) else {
        panic!("the first launch should hold the pipe");
    };
    let (requests, received) = mpsc::channel();
    primary.serve(move |files| {
        let _ = requests.send(files);
    });

    let files = vec![
        PathBuf::from(r"C:\Users\someone\notes.md"),
        PathBuf::from(r"D:\Projects\Résumé – 2026.md"),
    ];
    assert_eq!(second_launch(&name, &files), "handed-over");
    let got = received.recv_timeout(Duration::from_secs(30)).unwrap();
    assert_eq!(got, files);

    // A launch with no file hands over too: the first brings a window forward.
    assert_eq!(second_launch(&name, &[]), "handed-over");
    let got = received.recv_timeout(Duration::from_secs(30)).unwrap();
    assert!(got.is_empty());
}

#[test]
fn with_no_other_the_launch_is_the_first() {
    let name = test_pipe("first");
    assert_eq!(second_launch(&name, &[]), "primary");
}

#[test]
fn the_users_pipe_is_named_for_them_and_their_session() {
    let name = instance::pipe_name("Legible").unwrap();
    assert!(name.starts_with(r"\\.\pipe\Legible-S-1-"), "{name}");
}
