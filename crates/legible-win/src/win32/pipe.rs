//! A named pipe for one process to hand requests to another of the same user's: the server
//! side claims the name (only one process can), the client side connects and talks to it.
//!
//! Messages are whole (message mode), each up to [`MAX_MESSAGE`] bytes. Only the user who made
//! the pipe can open it (its security descriptor names their SID alone), and only from this PC.

use std::io;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_MORE_DATA, ERROR_PIPE_BUSY, ERROR_PIPE_CONNECTED,
    GENERIC_READ, GENERIC_WRITE, HANDLE, HLOCAL, INVALID_HANDLE_VALUE, LocalFree,
};
use windows::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE,
    FlushFileBuffers, OPEN_EXISTING, PIPE_ACCESS_DUPLEX, ReadFile, WriteFile,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, GetNamedPipeServerProcessId,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
    SetNamedPipeHandleState, WaitNamedPipeW,
};
use windows::core::HSTRING;

use super::{current_user_sid, io_error};

/// The longest message either side reads.
pub const MAX_MESSAGE: usize = 1024 * 1024;

const BUFFER: u32 = 64 * 1024;

/// A handle closed on drop.
struct Handle(HANDLE);

// SAFETY: a pipe handle may be used from any thread; each is used by one thread at a time.
unsafe impl Send for Handle {}

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: the handle is live and closed only here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// The server side: this process holds the name.
pub struct Server {
    handle: Handle,
}

impl Server {
    /// Claims `name` (`\\.\pipe\…`): `Ok(None)` when another process already holds it.
    pub fn claim(name: &str) -> io::Result<Option<Server>> {
        let sid = current_user_sid()?;
        // Generic all for this user only; P: protected, so nothing is inherited.
        let sddl = HSTRING::from(format!("D:P(A;;GA;;;{sid})"));
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the string is live and null-terminated; Windows allocates the descriptor,
        // which is freed once below, after the pipe is made.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|e| io_error(&e))?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: false.into(),
        };
        let name = HSTRING::from(name);
        // SAFETY: the name and the attributes (and the descriptor they point to) are live for
        // the call.
        let handle = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                BUFFER,
                BUFFER,
                0,
                Some(&attributes),
            )
        };
        let error = io::Error::last_os_error();
        // SAFETY: frees the descriptor Windows allocated, once.
        unsafe { LocalFree(Some(HLOCAL(descriptor.0))) };
        if handle == INVALID_HANDLE_VALUE {
            // Another process's first instance: access denied (the name exists) or busy.
            return match error.raw_os_error().map(|c| c as u32) {
                Some(c) if c == ERROR_ACCESS_DENIED.0 || c == ERROR_PIPE_BUSY.0 => Ok(None),
                _ => Err(error),
            };
        }
        Ok(Some(Server {
            handle: Handle(handle),
        }))
    }

    /// Waits for the next client and returns its connection. One client at a time: the
    /// connection ends (and the next can connect) when it's dropped.
    pub fn accept(&mut self) -> io::Result<Connection<'_>> {
        // SAFETY: the handle is live; no overlapped I/O.
        match unsafe { ConnectNamedPipe(self.handle.0, None) } {
            Ok(()) => {}
            // The client connected between the pipe's creation and this call: still connected.
            Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => {}
            Err(e) => return Err(io_error(&e)),
        }
        Ok(Connection {
            handle: &self.handle,
        })
    }
}

/// One client's connection to the [`Server`].
pub struct Connection<'a> {
    handle: &'a Handle,
}

impl Connection<'_> {
    pub fn read(&self) -> io::Result<Vec<u8>> {
        read_message(&self.handle.0)
    }

    pub fn write(&self, message: &[u8]) -> io::Result<()> {
        write_message(&self.handle.0, message)
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        // SAFETY: the handle is live; the client has what was written before the disconnect.
        unsafe {
            let _ = FlushFileBuffers(self.handle.0);
            let _ = DisconnectNamedPipe(self.handle.0);
        }
    }
}

/// The client side, connected to the process that holds the name.
pub struct Client {
    handle: Handle,
}

impl Client {
    /// Connects to `name`, waiting up to `wait_ms` while the server is busy with another
    /// client. `NotFound` when no process holds the name.
    pub fn connect(name: &str, wait_ms: u32) -> io::Result<Client> {
        let name = HSTRING::from(name);
        let handle = loop {
            // SAFETY: the name is live and null-terminated; no security attributes or template.
            match unsafe {
                CreateFileW(
                    &name,
                    (GENERIC_READ | GENERIC_WRITE).0,
                    FILE_SHARE_NONE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                )
            } {
                Ok(handle) => break Handle(handle),
                Err(e) if e.code() == ERROR_PIPE_BUSY.to_hresult() => {
                    // SAFETY: the name is live; it waits for the server to be free.
                    if !unsafe { WaitNamedPipeW(&name, wait_ms) }.as_bool() {
                        return Err(io::Error::new(io::ErrorKind::TimedOut, "the pipe is busy"));
                    }
                }
                Err(e) => return Err(io_error(&e)),
            }
        };
        let mode = PIPE_READMODE_MESSAGE;
        // SAFETY: the handle is live; `mode` outlives the call.
        unsafe { SetNamedPipeHandleState(handle.0, Some(&mode), None, None) }
            .map_err(|e| io_error(&e))?;
        Ok(Client { handle })
    }

    /// The process ID of the server.
    pub fn server_pid(&self) -> io::Result<u32> {
        let mut pid = 0u32;
        // SAFETY: the handle is live; `pid` is a live out pointer.
        unsafe { GetNamedPipeServerProcessId(self.handle.0, &mut pid) }
            .map_err(|e| io_error(&e))?;
        Ok(pid)
    }

    pub fn read(&self) -> io::Result<Vec<u8>> {
        read_message(&self.handle.0)
    }

    pub fn write(&self, message: &[u8]) -> io::Result<()> {
        write_message(&self.handle.0, message)
    }
}

fn read_message(handle: &HANDLE) -> io::Result<Vec<u8>> {
    let mut message = Vec::new();
    let mut chunk = vec![0u8; BUFFER as usize];
    loop {
        let mut read = 0u32;
        // SAFETY: the handle is live; `chunk` is a live buffer whose length is passed; `read`
        // is a live out pointer.
        let result = unsafe { ReadFile(*handle, Some(&mut chunk), Some(&mut read), None) };
        message.extend_from_slice(&chunk[..read as usize]);
        if message.len() > MAX_MESSAGE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the message is too long",
            ));
        }
        match result {
            Ok(()) => return Ok(message),
            Err(e) if e.code() == ERROR_MORE_DATA.to_hresult() => continue,
            Err(e) => return Err(io_error(&e)),
        }
    }
}

fn write_message(handle: &HANDLE, message: &[u8]) -> io::Result<()> {
    if message.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the message is too long",
        ));
    }
    let mut written = 0u32;
    // SAFETY: the handle is live; `message` is a live slice whose length is passed.
    unsafe { WriteFile(*handle, Some(message), Some(&mut written), None) }
        .map_err(|e| io_error(&e))?;
    if written as usize != message.len() {
        return Err(io::Error::new(
            io::ErrorKind::WriteZero,
            "the message was cut short",
        ));
    }
    Ok(())
}
