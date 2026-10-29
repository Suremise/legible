//! WebView2's request filter for [`crate::consent`]: on every request of every source (the page,
//! its frames, workers), before the window's page loads. A request that isn't Legible's own and
//! isn't a consented picture is answered with the window's denial response (a 403, made once at
//! install), so it never leaves.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::io;
use std::rc::Rc;

use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT, COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE, COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
    ICoreWebView2, ICoreWebView2_2, ICoreWebView2_22, ICoreWebView2WebResourceRequestedEventArgs,
    ICoreWebView2WebResourceResponse,
};
use webview2_com::{WebResourceRequestedEventHandler, take_pwstr};
use windows::core::{HSTRING, Interface, PWSTR};

use super::io_error;
use crate::consent::{self, Context, Exchange, FilterHost, Outcome, Request};

/// What a refusal that couldn't be made calls (the window then fails small).
pub type OnRefusalFailed = Box<dyn Fn()>;

/// Installs the filter on `core`: requests are let through only as `allowed` says at the time,
/// and `failed` hears of any request that couldn't be refused. Call it before the page loads.
pub fn install(
    core: &ICoreWebView2,
    allowed: Rc<RefCell<HashSet<String>>>,
    failed: OnRefusalFailed,
) -> io::Result<()> {
    let mut host = Host {
        core: core.clone(),
        allowed,
        failed: Some(failed),
        denial: None,
        fail_refusals: None,
    };
    consent::install(&mut host)
}

/// As [`install`], for the `remote-images` check only (the `checks` feature, which nothing
/// shipped turns on): while `fail_refusals` is set, a refusal fails as if WebView2 had turned
/// down its response, so the request goes ahead and the window must fail small.
#[cfg(feature = "checks")]
pub fn install_for_checks(
    core: &ICoreWebView2,
    allowed: Rc<RefCell<HashSet<String>>>,
    failed: OnRefusalFailed,
    fail_refusals: Rc<Cell<bool>>,
) -> io::Result<()> {
    let mut host = Host {
        core: core.clone(),
        allowed,
        failed: Some(failed),
        denial: None,
        fail_refusals: Some(fail_refusals),
    };
    consent::install(&mut host)
}

struct Host {
    core: ICoreWebView2,
    allowed: Rc<RefCell<HashSet<String>>>,
    failed: Option<OnRefusalFailed>,
    denial: Option<ICoreWebView2WebResourceResponse>,
    /// Only ever set by [`install_for_checks`].
    fail_refusals: Option<Rc<Cell<bool>>>,
}

impl FilterHost for Host {
    fn filters_every_source(&self) -> bool {
        self.core.cast::<ICoreWebView2_22>().is_ok()
    }

    fn make_denial(&mut self) -> io::Result<()> {
        // SAFETY: COM calls on the window's thread, on objects WebView2 gave us.
        let denial = unsafe {
            let environment = self
                .core
                .cast::<ICoreWebView2_2>()
                .and_then(|c| c.Environment())
                .map_err(|e| io_error(&e))?;
            environment
                .CreateWebResourceResponse(None, 403, &HSTRING::from("Forbidden"), &HSTRING::new())
                .map_err(|e| io_error(&e))?
        };
        self.denial = Some(denial);
        Ok(())
    }

    fn add_handler(&mut self) -> io::Result<()> {
        let denial = self
            .denial
            .clone()
            .ok_or_else(|| io::Error::other("no denial response"))?;
        let allowed = self.allowed.clone();
        let failed = self
            .failed
            .take()
            .ok_or_else(|| io::Error::other("installed twice"))?;
        let fail_refusals = self.fail_refusals.clone();
        let handler = WebResourceRequestedEventHandler::create(Box::new(move |_, args| {
            // No args: nothing to answer, and nothing WebView2 will send.
            let Some(args) = args else { return Ok(()) };
            let exchange = Args {
                args,
                denial: &denial,
                fail: fail_refusals.as_ref().is_some_and(|f| f.get()),
            };
            if consent::filter(&exchange, &allowed.borrow()) == Outcome::RefusalFailed {
                failed();
            }
            Ok(())
        }));
        let mut token = Default::default();
        // SAFETY: as above; the handler is WebView2's to keep.
        unsafe { self.core.add_WebResourceRequested(&handler, &mut token) }
            .map_err(|e| io_error(&e))
    }

    fn add_filter(&mut self) -> io::Result<()> {
        let core = self
            .core
            .cast::<ICoreWebView2_22>()
            .map_err(|e| io_error(&e))?;
        // SAFETY: as above.
        unsafe {
            core.AddWebResourceRequestedFilterWithRequestSourceKinds(
                &HSTRING::from("*"),
                COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL,
                COREWEBVIEW2_WEB_RESOURCE_REQUEST_SOURCE_KINDS_ALL,
            )
        }
        .map_err(|e| io_error(&e))
    }
}

/// One request's event, as [`Exchange`] reads and refuses it.
struct Args<'a> {
    args: ICoreWebView2WebResourceRequestedEventArgs,
    denial: &'a ICoreWebView2WebResourceResponse,
    /// A check's stand-in for WebView2 turning the response down.
    fail: bool,
}

impl Exchange for Args<'_> {
    fn read(&self) -> io::Result<Request> {
        // SAFETY: the event's own objects, on the window's thread; each string WebView2 hands
        // over is copied and freed by take_pwstr.
        unsafe {
            let request = self.args.Request().map_err(|e| io_error(&e))?;
            let mut uri = PWSTR::null();
            request.Uri(&mut uri).map_err(|e| io_error(&e))?;
            let uri = take_pwstr(uri);
            let mut method = PWSTR::null();
            request.Method(&mut method).map_err(|e| io_error(&e))?;
            let method = take_pwstr(method);
            let mut context = COREWEBVIEW2_WEB_RESOURCE_CONTEXT::default();
            self.args
                .ResourceContext(&mut context)
                .map_err(|e| io_error(&e))?;
            let context = if context == COREWEBVIEW2_WEB_RESOURCE_CONTEXT_IMAGE {
                Context::Image
            } else {
                Context::Other
            };
            Ok(Request {
                uri,
                context,
                method,
            })
        }
    }

    fn refuse(&self) -> io::Result<()> {
        if self.fail {
            return Err(io::Error::other("refusal made to fail by a check"));
        }
        // SAFETY: as above; the denial response was made for this WebView2 at install.
        unsafe { self.args.SetResponse(self.denial) }.map_err(|e| io_error(&e))
    }
}
