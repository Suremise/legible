//! One HTTP request with WinHTTP, part of Windows (so no HTTP or TLS crate ships in Legible): a
//! `GET` whose redirect is read, not followed. Only the update check uses it
//! ([`crate::update`]), and only when the reader has turned it on.

use std::time::Duration;

use windows::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_DISABLE_REDIRECTS, WINHTTP_FLAG_SECURE,
    WINHTTP_OPEN_REQUEST_FLAGS, WINHTTP_OPTION_DISABLE_FEATURE, WINHTTP_QUERY_FLAG_NUMBER,
    WINHTTP_QUERY_LOCATION, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReceiveResponse,
    WinHttpSendRequest, WinHttpSetOption, WinHttpSetTimeouts,
};
use windows::core::{HSTRING, PCWSTR, w};

/// A WinHTTP handle, closed when dropped.
struct Handle(*mut std::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: our own handle, closed once.
        let _ = unsafe { WinHttpCloseHandle(self.0) };
    }
}

/// Why a request failed: WinHTTP's error code (12002 for a timeout, 12007 for a name that
/// couldn't be found, 12029 for no connection, and so on), and Windows' words for it, which are
/// empty for WinHTTP's own codes (their text isn't in the system's table).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    pub code: u32,
    pub message: String,
}

impl From<windows::core::Error> for Failure {
    fn from(error: windows::core::Error) -> Self {
        Failure {
            // A Win32 error as an HRESULT: its code is the low word.
            code: (error.code().0 as u32) & 0xFFFF,
            message: error.message().trim().to_owned(),
        }
    }
}

/// WinHTTP's codes, as [`Failure::code`] has them.
pub const TIMEOUT: u32 = 12002;
pub const NAME_NOT_RESOLVED: u32 = 12007;
pub const CANNOT_CONNECT: u32 = 12029;
pub const CONNECTION_ERROR: u32 = 12030;

fn handle(raw: *mut std::ffi::c_void) -> Result<Handle, Failure> {
    if raw.is_null() {
        Err(windows::core::Error::from_thread().into())
    } else {
        Ok(Handle(raw))
    }
}

fn check(result: windows::core::Result<()>) -> Result<(), Failure> {
    result.map_err(Failure::from)
}

/// Where the request goes: HTTPS on 443 for the update check; a test's own server on the
/// loopback address takes plain HTTP on a port of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target<'a> {
    pub host: &'a str,
    pub port: u16,
    pub secure: bool,
}

/// What came back: its status, and its `Location` if it has one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    pub status: u32,
    pub location: Option<String>,
}

/// `GET <target><path>` with redirects off, as `agent`: the status and the `Location`. Nothing
/// but the request line and WinHTTP's own headers (the host, the agent) is sent, and no body
/// is read.
pub fn redirect_location(
    target: Target<'_>,
    path: &str,
    agent: &str,
    timeout: Duration,
) -> Result<Answer, Failure> {
    let (agent, host, path) = (
        HSTRING::from(agent),
        HSTRING::from(target.host),
        HSTRING::from(path),
    );
    let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
    let flags = if target.secure {
        WINHTTP_FLAG_SECURE
    } else {
        WINHTTP_OPEN_REQUEST_FLAGS(0)
    };
    // SAFETY: each handle is checked and closed by `Handle`; the strings outlive the calls;
    // the buffers passed to WinHttpQueryHeaders are live and their sizes given.
    unsafe {
        let session = handle(WinHttpOpen(
            &agent,
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ))?;
        check(WinHttpSetTimeouts(session.0, ms, ms, ms, ms))?;
        let connect = handle(WinHttpConnect(session.0, &host, target.port, 0))?;
        let request = handle(WinHttpOpenRequest(
            connect.0,
            w!("GET"),
            &path,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            flags,
        ))?;
        let no_redirects = WINHTTP_DISABLE_REDIRECTS;
        check(WinHttpSetOption(
            Some(request.0),
            WINHTTP_OPTION_DISABLE_FEATURE,
            Some(&no_redirects.to_ne_bytes()),
        ))?;
        check(WinHttpSendRequest(request.0, None, None, 0, 0, 0))?;
        check(WinHttpReceiveResponse(request.0, std::ptr::null_mut()))?;

        let mut status = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        check(WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some((&mut status as *mut u32).cast()),
            &mut size,
            std::ptr::null_mut(),
        ))?;
        let mut buffer = vec![0u16; 2048];
        let mut bytes = (buffer.len() * 2) as u32;
        let found = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_LOCATION,
            PCWSTR::null(),
            Some(buffer.as_mut_ptr().cast()),
            &mut bytes,
            std::ptr::null_mut(),
        );
        let location = found.is_ok().then(|| {
            let len = (bytes as usize / 2).min(buffer.len());
            String::from_utf16_lossy(&buffer[..len])
        });
        Ok(Answer { status, location })
    }
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::thread;

    use super::*;

    /// A server of the test's own on the loopback address that answers one request with
    /// `response`; the request it got comes back from the thread.
    fn serve_once(response: &'static str) -> (u16, thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                request.push_str(&line);
            }
            let mut stream = stream;
            stream.write_all(response.as_bytes()).unwrap();
            request
        });
        (port, server)
    }

    fn local(port: u16) -> Target<'static> {
        Target {
            host: "127.0.0.1",
            port,
            secure: false,
        }
    }

    #[test]
    fn a_redirect_is_read_not_followed() {
        let (port, server) = serve_once(
            "HTTP/1.1 302 Found\r\nLocation: https://github.com/Suremise/legible/releases/tag/v9.1.0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
        let answer = redirect_location(
            local(port),
            "/Suremise/legible/releases/latest",
            "Legible/test",
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!(answer.status, 302);
        assert_eq!(
            answer.location.as_deref(),
            Some("https://github.com/Suremise/legible/releases/tag/v9.1.0")
        );
        // The request is the path and WinHTTP's own headers: nothing else is sent.
        let request = server.join().unwrap();
        assert!(
            request.starts_with("GET /Suremise/legible/releases/latest HTTP/1.1\r\n"),
            "{request}"
        );
        assert!(
            request.contains("User-Agent: Legible/test\r\n"),
            "{request}"
        );
        let names: Vec<&str> = request
            .lines()
            .skip(1)
            .filter_map(|l| l.split_once(':').map(|(name, _)| name))
            .collect();
        for name in &names {
            assert!(
                ["Host", "User-Agent", "Connection"].contains(name),
                "an unexpected header, {name}: {request}"
            );
        }
    }

    #[test]
    fn an_answer_without_a_redirect_has_no_location() {
        let (port, server) =
            serve_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        let answer =
            redirect_location(local(port), "/x", "Legible/test", Duration::from_secs(10)).unwrap();
        assert_eq!(
            answer,
            Answer {
                status: 404,
                location: None
            }
        );
        server.join().unwrap();
    }

    #[test]
    fn no_server_is_cannot_connect() {
        // A port nothing listens on: bound, then let go.
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let error = redirect_location(local(port), "/x", "Legible/test", Duration::from_secs(10))
            .unwrap_err();
        assert_eq!(error.code, CANNOT_CONNECT, "{error:?}");
    }
}
