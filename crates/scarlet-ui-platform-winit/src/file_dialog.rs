//! AppKit sheets; all entry/poll/drop operations run on the winit UI thread.
use block2::RcBlock;
use objc2::rc::Retained;
use objc2_app_kit::{NSModalResponseCancel, NSModalResponseOK, NSOpenPanel, NSSavePanel, NSView};
use objc2_foundation::{MainThreadMarker, NSArray, NSString, NSURL};
use scarlet_ui_core::file_dialog::*;
use std::{cell::RefCell, path::PathBuf, rc::Rc};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::Window,
};

enum Panel {
    Open(Retained<NSOpenPanel>),
    Save(Retained<NSSavePanel>),
}
impl Panel {
    fn save_panel(&self) -> &NSSavePanel {
        match self {
            Self::Open(p) => p,
            Self::Save(p) => p,
        }
    }
}
struct Session {
    panel: Rc<Panel>,
    result: Rc<RefCell<Option<FileDialogResult>>>,
    finished: bool,
}
impl FileDialogSession for Session {
    fn poll(&mut self) -> Option<FileDialogResult> {
        let result = self.result.borrow_mut().take();
        self.finished |= result.is_some();
        result
    }
    fn cancel(&mut self) {
        if !self.finished {
            // SAFETY: the runner owns this session on the main thread.
            unsafe { self.panel.save_panel().cancel(None) };
            self.finished = true;
        }
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.cancel();
    }
}

pub(crate) fn begin(
    window: &Window,
    options: &FileDialog,
) -> Result<Box<dyn FileDialogSession>, FileDialogError> {
    options.validate()?;
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| FileDialogError::Platform("AppKit requires the main thread".into()))?;
    let raw = window
        .window_handle()
        .map_err(|e| FileDialogError::Platform(e.to_string()))?;
    let RawWindowHandle::AppKit(raw) = raw.as_raw() else {
        return Err(FileDialogError::Platform("owner has no AppKit view".into()));
    };
    // SAFETY: winit keeps the NSView alive; the runner holds the owner throughout
    // this call. The AppKit API and the completion block execute on the main thread.
    unsafe {
        let view = &*raw.ns_view.as_ptr().cast::<NSView>();
        let owner = view.window().ok_or(FileDialogError::OwnerClosed)?;
        if owner.attachedSheet().is_some() {
            return Err(FileDialogError::Busy);
        }
        let panel = Rc::new(match options.mode {
            FileDialogMode::Open | FileDialogMode::OpenMultiple => {
                let p = NSOpenPanel::openPanel(mtm);
                p.setCanChooseFiles(true);
                p.setCanChooseDirectories(false);
                p.setAllowsMultipleSelection(options.mode == FileDialogMode::OpenMultiple);
                Panel::Open(p)
            }
            FileDialogMode::Save => Panel::Save(NSSavePanel::savePanel(mtm)),
            _ => return Err(FileDialogError::Unsupported("unknown operation".into())),
        });
        let p = panel.save_panel();
        if !options.title.is_empty() {
            p.setTitle(Some(&NSString::from_str(&options.title)));
        }
        if let Some(dir) = &options.initial_directory {
            // NSURL file-system representation preserves non-UTF8 paths.
            use std::os::unix::ffi::OsStrExt;
            let path = std::ffi::CString::new(dir.as_path().as_os_str().as_bytes())
                .map_err(|_| FileDialogError::InvalidOptions("directory contains NUL".into()))?;
            let url = NSURL::fileURLWithFileSystemRepresentation_isDirectory_relativeToURL(
                core::ptr::NonNull::new(path.as_ptr().cast_mut()).unwrap(),
                true,
                None,
            );
            p.setDirectoryURL(Some(&url));
        }
        if let Some(name) = &options.default_name {
            p.setNameFieldStringValue(&NSString::from_str(name));
        }
        p.setCanCreateDirectories(true);
        if !options.filters.is_empty() {
            let extensions: Vec<_> = options
                .filters
                .iter()
                .flat_map(|f| f.extensions.iter())
                .map(|e| NSString::from_str(e))
                .collect();
            let types = NSArray::from_id_slice(&extensions);
            // This older binding exposes the extension-based API. AppKit applies
            // the union and owns overwrite confirmation/default extension behavior.
            #[allow(deprecated)]
            p.setAllowedFileTypes(Some(&types));
            p.setAllowsOtherFileTypes(false);
        }
        let result = Rc::new(RefCell::new(None));
        let result_callback = result.clone();
        // Weak avoids panel -> block -> panel retention cycles.
        let weak = Rc::downgrade(&panel);
        let block = RcBlock::new(move |response| {
            let value = if response == NSModalResponseCancel {
                Ok(FileDialogOutcome::Cancelled)
            } else if response == NSModalResponseOK {
                weak.upgrade()
                    .ok_or(FileDialogError::OwnerClosed)
                    .and_then(|panel| {
                        let urls = match panel.as_ref() {
                            Panel::Open(p) => p.URLs(),
                            Panel::Save(p) => {
                                let url = p.URL().ok_or_else(|| {
                                    FileDialogError::Platform("save panel returned no URL".into())
                                })?;
                                NSArray::from_id_slice(&[url])
                            }
                        };
                        let mut paths = Vec::new();
                        for i in 0..urls.count() {
                            let url = urls.objectAtIndex(i);
                            use std::os::unix::ffi::OsStrExt;
                            let repr = url.fileSystemRepresentation();
                            let bytes = std::ffi::CStr::from_ptr(repr.as_ptr()).to_bytes();
                            paths.push(FileDialogPath::from(PathBuf::from(
                                std::ffi::OsStr::from_bytes(bytes),
                            )));
                        }
                        if paths.is_empty() {
                            Err(FileDialogError::Platform(
                                "panel returned empty selection".into(),
                            ))
                        } else {
                            Ok(FileDialogOutcome::Selected(paths))
                        }
                    })
            } else {
                Err(FileDialogError::Platform(format!(
                    "unexpected AppKit response: {response}"
                )))
            };
            *result_callback.borrow_mut() = Some(value);
        });
        p.beginSheetModalForWindow_completionHandler(&owner, &block);
        Ok(Box::new(Session {
            panel,
            result,
            finished: false,
        }))
    }
}
