//! Windows' list of custom destinations (`ICustomDestinationList`) for [`crate::jumplist`],
//! and the process's app identity. COM is already set up on the UI thread (tao makes it a
//! single-threaded apartment), where these are called.

use std::ffi::OsString;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use windows::Win32::Storage::EnhancedStorage::PKEY_Title;
use windows::Win32::System::Com::StructuredStorage::PROPVARIANT;
use windows::Win32::System::Com::{CLSCTX_INPROC_SERVER, CoCreateInstance};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW,
    SetCurrentProcessExplicitAppUserModelID, ShellLink,
};
use windows::core::{HSTRING, Interface};

use super::{io_error, wide};
use crate::jumplist::{Destinations, Link};

/// Gives this process `app_id`, so its taskbar button and Jump List are Legible's. Called
/// before any window is made.
pub fn set_app_id(app_id: &str) -> io::Result<()> {
    // SAFETY: the string is live and null-terminated for the call.
    unsafe { SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(app_id)) }
        .map_err(|e| io_error(&e))
}

/// The custom destination list of `app_id`.
pub struct ShellDestinations {
    list: ICustomDestinationList,
}

impl ShellDestinations {
    pub fn new(app_id: &str) -> io::Result<Self> {
        // SAFETY: COM is initialised on this thread; the class and interface are Windows' own.
        let list: ICustomDestinationList =
            unsafe { CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER) }
                .map_err(|e| io_error(&e))?;
        // SAFETY: as above; the string lives for the call.
        unsafe { list.SetAppID(&HSTRING::from(app_id)) }.map_err(|e| io_error(&e))?;
        Ok(Self { list })
    }

    /// Deletes `app_id`'s list: for a probe cleaning up after itself.
    pub fn delete(&self, app_id: &str) -> io::Result<()> {
        // SAFETY: as in `new`.
        unsafe { self.list.DeleteList(&HSTRING::from(app_id)) }.map_err(|e| io_error(&e))
    }
}

fn wide_text(buffer: &[u16]) -> OsString {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    OsString::from_wide(&buffer[..len])
}

impl Destinations for ShellDestinations {
    fn begin(&mut self) -> io::Result<Vec<Link>> {
        let mut slots = 0u32;
        // SAFETY: COM on this thread; `slots` is ours to fill.
        let removed: IObjectArray =
            unsafe { self.list.BeginList(&mut slots) }.map_err(|e| io_error(&e))?;
        // SAFETY: the array Windows returned, read within its count.
        let count = unsafe { removed.GetCount() }.map_err(|e| io_error(&e))?;
        let mut links = Vec::new();
        for i in 0..count {
            // Anything that isn't a shell link isn't one of ours.
            // SAFETY: as above, within the count.
            let Ok(link) = (unsafe { removed.GetAt::<IShellLinkW>(i) }) else {
                continue;
            };
            let mut path = vec![0u16; 32_768];
            let mut arguments = vec![0u16; 32_768];
            // SAFETY: the buffers are ours, their lengths given by the slices; no find data.
            let read = unsafe {
                link.GetPath(&mut path, std::ptr::null_mut(), 0)
                    .and_then(|()| link.GetArguments(&mut arguments))
            };
            if read.is_ok() {
                links.push(Link {
                    exe: PathBuf::from(wide_text(&path)),
                    arguments: wide_text(&arguments),
                    title: OsString::new(),
                });
            }
        }
        Ok(links)
    }

    fn append(&mut self, category: &str, links: &[Link]) -> io::Result<()> {
        // SAFETY: COM on this thread; every string below lives for its call.
        unsafe {
            let collection: IObjectCollection =
                CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)
                    .map_err(|e| io_error(&e))?;
            for link in links {
                let shell_link: IShellLinkW =
                    CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)
                        .map_err(|e| io_error(&e))?;
                shell_link
                    .SetPath(&wide(&link.exe))
                    .map_err(|e| io_error(&e))?;
                shell_link
                    .SetArguments(&HSTRING::from(link.arguments.as_os_str()))
                    .map_err(|e| io_error(&e))?;
                shell_link
                    .SetIconLocation(&wide(&link.exe), 0)
                    .map_err(|e| io_error(&e))?;
                let store: IPropertyStore = shell_link.cast().map_err(|e| io_error(&e))?;
                // Only shown, never compared or stored: a name that isn't valid Unicode shows
                // its replacement characters.
                let title = PROPVARIANT::from(link.title.to_string_lossy().as_ref());
                store
                    .SetValue(&PKEY_Title, &title)
                    .map_err(|e| io_error(&e))?;
                store.Commit().map_err(|e| io_error(&e))?;
                collection
                    .AddObject(&shell_link)
                    .map_err(|e| io_error(&e))?;
            }
            let array: IObjectArray = collection.cast().map_err(|e| io_error(&e))?;
            self.list
                .AppendCategory(&HSTRING::from(category), &array)
                .map_err(|e| io_error(&e))
        }
    }

    fn commit(&mut self) -> io::Result<()> {
        // SAFETY: COM on this thread.
        unsafe { self.list.CommitList() }.map_err(|e| io_error(&e))
    }

    fn abort(&mut self) {
        // SAFETY: COM on this thread. Failing leaves the old list, which is what aborting wants.
        let _ = unsafe { self.list.AbortList() };
    }
}
