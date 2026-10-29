//! One process per user: the first Legible to start holds a named pipe, and every later
//! launch hands its files to it and exits, so each file has one window and all of them share
//! one process.
//!
//! A launch [`claim`]s the user's pipe. If it gets it, it is the [`Primary`]: it serves the
//! pipe on a thread of its own and is given each later launch's request. If another process
//! holds it, the launch checks that process is the same user's, lets it take the foreground,
//! sends it the files and waits for its answer: then it has [`HandedOver`](Claim::HandedOver).
//! A primary that has just exited, or one that doesn't answer, leaves the launch to carry on
//! as a primary of its own (without the pipe if it can't get it), so a file always opens.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use crate::win32::{self, pipe};

/// How long a launch waits for the primary to take its request.
const HANDOVER_TIMEOUT: Duration = Duration::from_secs(5);
/// How often a launch tries again when the pipe changes hands as it connects.
const ATTEMPTS: usize = 5;
/// The primary's answer once it has the request.
const ACK: &[u8] = b"ok";

/// The user's pipe for `app`: one per user and Windows session.
pub fn pipe_name(app: &str) -> io::Result<String> {
    let sid = win32::current_user_sid()?;
    let session = win32::current_session_id()?;
    Ok(format!(r"\\.\pipe\{app}-{sid}-{session}"))
}

/// What a launch is.
pub enum Claim {
    /// The first: it opens the windows. `None` when the pipe couldn't be had, so later launches
    /// open their own.
    Primary(Option<Primary>),
    /// Another process has the files now: exit.
    HandedOver,
}

/// The process that holds the pipe, until it [`serve`](Primary::serve)s it.
pub struct Primary {
    server: pipe::Server,
}

/// Claims the pipe `name`, or hands `files` to the process that holds it.
pub fn claim(name: &str, files: &[PathBuf]) -> Claim {
    for _ in 0..ATTEMPTS {
        match pipe::Server::claim(name) {
            Ok(Some(server)) => return Claim::Primary(Some(Primary { server })),
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "couldn't make the single-instance pipe");
                return Claim::Primary(None);
            }
        }
        match hand_over(name, files) {
            Ok(()) => return Claim::HandedOver,
            // The primary exited between the two calls: try to be the primary.
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                tracing::warn!(%error, "couldn't hand the files to the open Legible");
                return Claim::Primary(None);
            }
        }
    }
    Claim::Primary(None)
}

/// Sends `files` to the primary and waits for its answer, on a thread so a primary that
/// doesn't answer can't hold this launch.
fn hand_over(name: &str, files: &[PathBuf]) -> io::Result<()> {
    let (name, message) = (name.to_owned(), encode(files));
    let (done, result) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(send(&name, &message));
    });
    result
        .recv_timeout(HANDOVER_TIMEOUT)
        .unwrap_or_else(|_| Err(io::Error::new(io::ErrorKind::TimedOut, "no answer")))
}

fn send(name: &str, message: &[u8]) -> io::Result<()> {
    let client = pipe::Client::connect(name, HANDOVER_TIMEOUT.as_millis() as u32)?;
    let pid = client.server_pid()?;
    // Only ever hand files to the same user's process: another user could have made a pipe
    // of this name first.
    let ours = win32::current_user_sid()?;
    if win32::user_sid_of_process(pid).ok().as_deref() != Some(ours.as_str()) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe is another user's",
        ));
    }
    win32::allow_foreground(pid);
    client.write(message)?;
    match client.read()? {
        answer if answer == ACK => Ok(()),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an unknown answer",
        )),
    }
}

impl Primary {
    /// Serves the pipe on a thread of its own: `on_request` is given each later launch's
    /// files (none when it was started without one), and runs on that thread.
    pub fn serve(mut self, on_request: impl Fn(Vec<PathBuf>) + Send + 'static) {
        std::thread::Builder::new()
            .name("single-instance".into())
            .spawn(move || {
                loop {
                    let connection = match self.server.accept() {
                        Ok(connection) => connection,
                        Err(error) => {
                            tracing::error!(%error, "the single-instance pipe failed");
                            return;
                        }
                    };
                    match connection.read() {
                        Ok(message) => {
                            on_request(decode(&message));
                            let _ = connection.write(ACK);
                        }
                        Err(error) => tracing::warn!(%error, "a launch's request was unreadable"),
                    }
                }
            })
            .expect("a thread for the single-instance pipe");
    }
}

/// The files as UTF-16 (Windows' own form, so any path survives), each ended by a null.
pub fn encode(files: &[PathBuf]) -> Vec<u8> {
    files
        .iter()
        .flat_map(|f| f.as_os_str().encode_wide().chain([0]))
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// The files [`encode`] wrote; an odd last byte or a missing last null is dropped.
pub fn decode(message: &[u8]) -> Vec<PathBuf> {
    let units: Vec<u16> = message
        .as_chunks::<2>()
        .0
        .iter()
        .map(|&b| u16::from_le_bytes(b))
        .collect();
    let mut files: Vec<PathBuf> = units
        .split(|&u| u == 0)
        .map(|f| PathBuf::from(OsString::from_wide(f)))
        .collect();
    // The text after the last null: empty for a whole message.
    files.pop();
    files.retain(|f| !f.as_os_str().is_empty());
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_survive_the_message() {
        let files = vec![
            PathBuf::from(r"C:\Users\me\notes.md"),
            PathBuf::from(r"\\server\share\Résumé – 2026.md"),
            PathBuf::from("日本語.md"),
        ];
        assert_eq!(decode(&encode(&files)), files);
        assert_eq!(decode(&encode(&[])), Vec::<PathBuf>::new());
    }

    #[test]
    fn a_broken_message_loses_only_its_broken_end() {
        let mut message = encode(&[PathBuf::from("a.md"), PathBuf::from("b.md")]);
        message.truncate(message.len() - 3);
        assert_eq!(decode(&message), vec![PathBuf::from("a.md")]);
        assert_eq!(decode(&[0x41]), Vec::<PathBuf>::new());
    }
}
