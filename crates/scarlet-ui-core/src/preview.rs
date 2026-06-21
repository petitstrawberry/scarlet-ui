//! Native desktop preview support for ScarletUI.
//!
//! Preview is intentionally separate from normal `Application::run()`.
//! It is a development-only Rust dylib boundary used by the preview host.

use alloc::boxed::Box;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::path::Path;
use std::time::Duration;

use crate::buffer::Buffer;
use crate::compositor::DamageRect;
use crate::element::{ComponentElement, Element, TextInputElementState};
use crate::error::Error;
use crate::event::{Event, WindowEvent};
use crate::geometry::{Alignment, Size};
use crate::pipeline::RenderingPipeline;
use crate::platform::{PlatformBackend, PlatformWindow, WindowCreateRequest};
use crate::view::View;
use crate::views::{AlignmentFrame, Window};

/// Preview entry symbol exported by preview dylibs.
pub const PREVIEW_ENTRY_SYMBOL: &[u8] = b"scarlet_ui_preview_entry";

/// Stable identifier for a preview entry inside a preview library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PreviewId(String);

impl PreviewId {
    /// Create a preview ID.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Borrow this ID as a string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for PreviewId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for PreviewId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// Metadata shown by the preview host before a session is created.
#[derive(Clone, Debug)]
pub struct PreviewDescriptor {
    /// Stable preview ID.
    pub id: PreviewId,
    /// Human-readable preview name.
    pub name: String,
}

impl PreviewDescriptor {
    /// Create preview metadata.
    pub fn new(id: impl Into<PreviewId>, name: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
        }
    }
}

/// Initial context passed to a preview session.
#[derive(Clone, Copy, Debug)]
pub struct PreviewCreateContext {
    /// Initial logical window size.
    pub size: Size,
    /// Output scale in milli-units.
    pub scale_milli: u32,
}

impl Default for PreviewCreateContext {
    fn default() -> Self {
        Self {
            size: Size::new(800.0, 600.0),
            scale_milli: 1000,
        }
    }
}

/// Rendered preview frame.
pub struct PreviewFrame<'a> {
    /// BGRA frame buffer.
    pub buffer: &'a Buffer,
    /// Physical damage rectangles, or `None` for full present.
    pub damage: Option<&'a [DamageRect]>,
}

/// A running preview instance.
pub trait PreviewSession {
    fn title(&self) -> &str;
    fn size(&self) -> Size;
    fn resize(&mut self, size: Size, scale_milli: u32);
    fn handle_event(&mut self, event: &Event) -> bool;
    fn take_emitted_events(&mut self) -> Vec<Event>;
    fn focused_text_input_state(&self) -> Option<TextInputElementState>;
    fn render(&mut self) -> Option<PreviewFrame<'_>>;
    fn set_paint_enabled(&mut self, _enabled: bool) {}
}

/// Preview library loaded from a Rust dylib.
pub trait PreviewLibrary {
    /// Return all previews exported by this dylib.
    fn previews(&self) -> Vec<PreviewDescriptor>;

    /// Create a preview session.
    fn create(
        &mut self,
        id: &PreviewId,
        context: PreviewCreateContext,
    ) -> Option<Box<dyn PreviewSession>>;
}

/// Statically registered preview factory.
pub struct PreviewRegistration {
    /// Stable preview ID.
    pub id: &'static str,
    /// Human-readable preview name.
    pub name: &'static str,
    /// Preferred logical preview size, or `Size::ZERO` for the host default.
    pub preferred_size: Size,
    /// Create a preview session.
    pub create: fn(PreviewCreateContext) -> Box<dyn PreviewSession>,
}

inventory::collect!(PreviewRegistration);

struct RegisteredPreviewLibrary;

impl PreviewLibrary for RegisteredPreviewLibrary {
    fn previews(&self) -> Vec<PreviewDescriptor> {
        inventory::iter::<PreviewRegistration>
            .into_iter()
            .map(|registration| PreviewDescriptor::new(registration.id, registration.name))
            .collect()
    }

    fn create(
        &mut self,
        id: &PreviewId,
        mut context: PreviewCreateContext,
    ) -> Option<Box<dyn PreviewSession>> {
        inventory::iter::<PreviewRegistration>
            .into_iter()
            .find(|registration| registration.id == id.as_str())
            .map(|registration| {
                if !registration.preferred_size.is_zero() {
                    context.size = registration.preferred_size;
                }
                (registration.create)(context)
            })
    }
}

/// Create a preview library from all registered preview entries.
pub fn registered_preview_library() -> Box<dyn PreviewLibrary> {
    Box::new(RegisteredPreviewLibrary)
}

struct SinglePreviewLibrary<F, V>
where
    F: Fn() -> V + 'static,
    V: View + Clone + 'static,
{
    descriptor: PreviewDescriptor,
    factory: F,
}

impl<F, V> PreviewLibrary for SinglePreviewLibrary<F, V>
where
    F: Fn() -> V + 'static,
    V: View + Clone + 'static,
{
    fn previews(&self) -> Vec<PreviewDescriptor> {
        alloc::vec![self.descriptor.clone()]
    }

    fn create(
        &mut self,
        id: &PreviewId,
        context: PreviewCreateContext,
    ) -> Option<Box<dyn PreviewSession>> {
        if id != &self.descriptor.id {
            return None;
        }
        Some(Box::new(ViewPreviewSession::new(
            self.descriptor.name.clone(),
            (self.factory)(),
            context,
        )))
    }
}

/// Create a preview library with a single preview entry.
pub fn single_preview_library<F, V>(name: &'static str, factory: F) -> Box<dyn PreviewLibrary>
where
    F: Fn() -> V + 'static,
    V: View + Clone + 'static,
{
    Box::new(SinglePreviewLibrary {
        descriptor: PreviewDescriptor::new(name, name),
        factory,
    })
}

/// Create a preview session from a `View`.
pub fn preview_session_from_view<V>(
    fallback_title: &'static str,
    view: V,
    context: PreviewCreateContext,
) -> Box<dyn PreviewSession>
where
    V: View + Clone + 'static,
{
    Box::new(ViewPreviewSession::new(
        fallback_title.to_string(),
        view,
        context,
    ))
}

struct ViewPreviewSession {
    title: String,
    pipeline: RenderingPipeline,
}

#[derive(Clone)]
struct PreviewShell<V: View + Clone> {
    title: String,
    size: Size,
    content: V,
}

impl<V: View + Clone> PreviewShell<V> {
    fn new(title: String, size: Size, content: V) -> Self {
        Self {
            title,
            size,
            content,
        }
    }
}

impl<V> View for PreviewShell<V>
where
    V: View + Clone + 'static,
{
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(ComponentElement::new_with_builder(
            self.clone(),
            Self::create_shell_element,
        ))
    }

    fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
        self.content.listenables()
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl<V> PreviewShell<V>
where
    V: View + Clone + 'static,
{
    fn create_shell_element(shell: &Self) -> Box<dyn Element> {
        Window::new(
            shell.title.clone(),
            AlignmentFrame::new(shell.content.clone(), Alignment::Center),
        )
        .size(shell.size)
        .create_element()
    }
}

impl ViewPreviewSession {
    fn new<V>(fallback_title: String, view: V, context: PreviewCreateContext) -> Self
    where
        V: View + Clone + 'static,
    {
        let mut pipeline = RenderingPipeline::new();
        pipeline.set_scale_milli(context.scale_milli);
        let requested_size = if context.size.width > 0.0 && context.size.height > 0.0 {
            context.size
        } else {
            PreviewCreateContext::default().size
        };
        pipeline.set_root(
            PreviewShell::new(fallback_title.clone(), requested_size, view).create_element(),
        );
        let window_info = pipeline.layout_initial();
        let mut title = window_info.title;
        if title == "ScarletUI Application" {
            title = fallback_title;
        }
        pipeline.resize(requested_size);
        Self { title, pipeline }
    }
}

impl PreviewSession for ViewPreviewSession {
    fn title(&self) -> &str {
        &self.title
    }

    fn size(&self) -> Size {
        self.pipeline.window_size()
    }

    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.pipeline.set_scale_milli(scale_milli);
        self.pipeline.resize(size);
    }

    fn handle_event(&mut self, event: &Event) -> bool {
        self.pipeline.handle_event(event)
    }

    fn take_emitted_events(&mut self) -> Vec<Event> {
        self.pipeline.take_emitted_events()
    }

    fn focused_text_input_state(&self) -> Option<TextInputElementState> {
        self.pipeline.focused_text_input_state()
    }

    fn render(&mut self) -> Option<PreviewFrame<'_>> {
        let (buffer, damage) = self.pipeline.render_with_damage()?;
        Some(PreviewFrame { buffer, damage })
    }

    fn set_paint_enabled(&mut self, enabled: bool) {
        self.pipeline.set_paint_enabled(enabled);
    }
}

/// Loaded preview dylib.
pub struct LoadedPreviewLibrary {
    api: Box<dyn PreviewLibrary>,
    _library: libloading::Library,
}

impl LoadedPreviewLibrary {
    /// Load a preview dylib.
    ///
    /// # Safety
    ///
    /// The dylib must be built by the same Rust toolchain and against the same
    /// ScarletUI preview API as the host. All preview objects created from this
    /// library must be dropped before this value is dropped.
    pub unsafe fn load(path: impl AsRef<Path>) -> core::result::Result<Self, String> {
        // SAFETY: The caller guarantees `path` points to a compatible preview
        // dylib whose lifetime is managed by `Self`.
        let library = unsafe { libloading::Library::new(path.as_ref()) }
            .map_err(|error| error.to_string())?;
        // SAFETY: The preview dylib contract requires this symbol to exist with
        // the exact `unsafe fn() -> Box<dyn PreviewLibrary>` signature.
        let entry = unsafe {
            library
                .get::<unsafe fn() -> Box<dyn PreviewLibrary>>(PREVIEW_ENTRY_SYMBOL)
                .map_err(|error| error.to_string())?
        };
        // SAFETY: The symbol ABI is validated by the preview contract above,
        // and the returned trait object cannot outlive `library`.
        let api = unsafe { entry() };
        drop(entry);
        Ok(Self {
            api,
            _library: library,
        })
    }

    /// Return all previews exported by this library.
    pub fn previews(&self) -> Vec<PreviewDescriptor> {
        self.api.previews()
    }

    /// Create a preview session.
    pub fn create(
        &mut self,
        id: &PreviewId,
        context: PreviewCreateContext,
    ) -> Option<Box<dyn PreviewSession>> {
        self.api.create(id, context)
    }
}

/// Preview host backed by a supplied platform backend.
pub struct PreviewHost {
    session: Option<Box<dyn PreviewSession>>,
    loaded: Option<LoadedPreviewLibrary>,
    window: Box<dyn PlatformWindow>,
    preview_id: PreviewId,
    scale_override_milli: Option<u32>,
    sync_after_reload: bool,
    full_present_frames: u8,
    gpu_present: Option<Box<dyn FnMut(&Buffer, Option<&[DamageRect]>)>>,
}

impl PreviewHost {
    /// Create a host from a selected preview exported by a dylib and a backend.
    pub fn new_with_backend(
        mut loaded: LoadedPreviewLibrary,
        preview: Option<&str>,
        backend: &mut dyn PlatformBackend,
        scale_override_milli: Option<u32>,
    ) -> core::result::Result<Self, String> {
        let descriptor = select_preview(loaded.previews(), preview)?;
        let scale_milli = scale_override_milli.unwrap_or_else(|| backend.output_scale_milli());
        let context = PreviewCreateContext {
            scale_milli,
            ..PreviewCreateContext::default()
        };
        let session = loaded
            .create(&descriptor.id, context)
            .ok_or_else(|| String::from("failed to create preview session"))?;
        let size = session.size();
        let mut window = backend
            .create_window(WindowCreateRequest {
                app_id: String::from("org.scarlet-os.scarletui.preview"),
                title: session.title().to_string(),
                size,
                window_type: 0,
                menu_titles: String::new(),
                focus_on_create: true,
                active_on_focus: true,
                opaque: true,
            })
            .map_err(|error| error.to_string())?;
        window.set_title(session.title());
        Ok(Self {
            session: Some(session),
            loaded: Some(loaded),
            window,
            preview_id: descriptor.id,
            scale_override_milli,
            sync_after_reload: true,
            full_present_frames: 2,
            gpu_present: None,
        })
    }

    pub fn set_gpu_present(&mut self, f: Box<dyn FnMut(&Buffer, Option<&[DamageRect]>)>) {
        self.gpu_present = Some(f);
    }

    pub fn window(&self) -> &dyn PlatformWindow {
        self.window.as_ref()
    }

    fn effective_scale_milli(&self) -> u32 {
        self.scale_override_milli
            .unwrap_or_else(|| self.window.output_scale_milli())
    }

    pub fn set_paint_enabled(&mut self, enabled: bool) {
        if let Some(session) = self.session.as_mut() {
            session.set_paint_enabled(enabled);
        }
    }

    /// Replace the loaded preview library after a successful rebuild.
    pub fn reload(&mut self, mut loaded: LoadedPreviewLibrary) -> core::result::Result<(), String> {
        let size = self.window.size();
        let scale_milli = self.effective_scale_milli();
        let session = loaded
            .create(&self.preview_id, PreviewCreateContext { size, scale_milli })
            .or_else(|| {
                let descriptor = loaded.previews().into_iter().next()?;
                self.preview_id = descriptor.id.clone();
                loaded.create(&descriptor.id, PreviewCreateContext { size, scale_milli })
            })
            .ok_or_else(|| String::from("failed to create reloaded preview session"))?;

        self.session = None;
        self.loaded = None;
        self.window.set_title(session.title());
        self.loaded = Some(loaded);
        self.session = Some(session);
        self.sync_after_reload = true;
        self.full_present_frames = 2;
        Ok(())
    }

    /// Switch to another preview exported by the currently loaded library.
    pub fn switch_preview(&mut self, preview: &str) -> core::result::Result<(), String> {
        let scale_override_milli = self.scale_override_milli;
        let loaded = self
            .loaded
            .as_mut()
            .ok_or_else(|| String::from("preview host has no loaded library"))?;
        let descriptor = select_preview(loaded.previews(), Some(preview))?;
        let size = self.window.size();
        let scale_milli = scale_override_milli.unwrap_or_else(|| self.window.output_scale_milli());
        let context = PreviewCreateContext { size, scale_milli };
        let session = loaded
            .create(&descriptor.id, context)
            .ok_or_else(|| String::from("failed to create switched preview session"))?;

        self.session = None;
        self.window.set_title(session.title());
        self.preview_id = descriptor.id;
        self.session = Some(session);
        self.sync_after_reload = true;
        self.full_present_frames = 2;
        Ok(())
    }

    /// Close the platform window and release the session and library.
    pub fn close(&mut self) {
        let _ = self.window.close();
        self.session = None;
        self.loaded = None;
    }

    /// Run one event/render tick. Returns `false` when the preview should exit.
    pub fn tick(&mut self, timeout: Duration) -> core::result::Result<bool, String> {
        let scale_override_milli = self.scale_override_milli;
        let Some(session) = self.session.as_mut() else {
            return Err(String::from("preview host has no active session"));
        };

        let mut had_event = false;
        while let Some(event) = self.window.poll_event() {
            had_event = true;
            if !Self::handle_event(
                self.window.as_mut(),
                session.as_mut(),
                event,
                self.scale_override_milli,
            )? {
                return Ok(false);
            }
        }

        self.window
            .sync_text_input(session.focused_text_input_state().as_ref());

        let size = self.window.size();
        let scale_milli = scale_override_milli.unwrap_or_else(|| self.window.output_scale_milli());

        if self.sync_after_reload || session.size() != size {
            session.resize(size, scale_milli);
            self.sync_after_reload = false;
        }

        if let Some(frame) = session.render() {
            let damage = if self.full_present_frames > 0 {
                self.full_present_frames -= 1;
                None
            } else {
                frame.damage
            };
            if let Some(gpu) = self.gpu_present.as_mut() {
                gpu(frame.buffer, damage);
            } else {
                self.window.present_with_damage(frame.buffer, damage);
            }
        }

        if !had_event {
            self.window.wait_for_event(timeout);
        }

        Ok(true)
    }

    fn handle_event(
        window: &mut dyn PlatformWindow,
        session: &mut dyn PreviewSession,
        event: Event,
        scale_override_milli: Option<u32>,
    ) -> core::result::Result<bool, String> {
        match event {
            Event::Quit => return Ok(false),
            Event::Resize { width, height } => {
                let size = Size::new(width as f32, height as f32);
                session.resize(
                    size,
                    scale_override_milli.unwrap_or_else(|| window.output_scale_milli()),
                );
            }
            event => {
                let _ = session.handle_event(&event);
            }
        }

        for emitted in session.take_emitted_events() {
            match emitted {
                Event::Window(WindowEvent::CloseRequested) => return Ok(false),
                Event::Window(WindowEvent::MinimizeRequested) => {
                    window.minimize().map_err(error_to_string)?;
                }
                Event::Window(WindowEvent::MaximizeRequested) => {
                    window.maximize().map_err(error_to_string)?;
                }
                Event::Window(WindowEvent::RestoreRequested) => {
                    window.restore().map_err(error_to_string)?;
                }
                Event::Window(WindowEvent::MoveRequested) => {
                    window.request_move().map_err(error_to_string)?;
                }
                _ => {}
            }
        }

        Ok(true)
    }
}

fn select_preview(
    previews: Vec<PreviewDescriptor>,
    preview: Option<&str>,
) -> core::result::Result<PreviewDescriptor, String> {
    let Some(preview) = preview else {
        return previews
            .into_iter()
            .next()
            .ok_or_else(|| String::from("preview library exports no previews"));
    };
    previews
        .iter()
        .find(|descriptor| descriptor.id.as_str() == preview || descriptor.name == preview)
        .cloned()
        .ok_or_else(|| format!("preview not found: {preview}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Point;

    struct TestLibrary {
        descriptors: Vec<PreviewDescriptor>,
    }

    impl TestLibrary {
        fn new() -> Self {
            Self {
                descriptors: alloc::vec![
                    PreviewDescriptor::new("button_preview", "Button Preview"),
                    PreviewDescriptor::new("counter_preview", "Counter Preview"),
                ],
            }
        }
    }

    impl PreviewLibrary for TestLibrary {
        fn previews(&self) -> Vec<PreviewDescriptor> {
            self.descriptors.clone()
        }

        fn create(
            &mut self,
            id: &PreviewId,
            _context: PreviewCreateContext,
        ) -> Option<Box<dyn PreviewSession>> {
            self.descriptors
                .iter()
                .find(|descriptor| &descriptor.id == id)
                .map(|descriptor| {
                    Box::new(TestSession {
                        title: descriptor.name.clone(),
                    }) as Box<dyn PreviewSession>
                })
        }
    }

    struct TestSession {
        title: String,
    }

    impl PreviewSession for TestSession {
        fn title(&self) -> &str {
            &self.title
        }

        fn size(&self) -> Size {
            Size::new(320.0, 240.0)
        }

        fn resize(&mut self, _size: Size, _scale_milli: u32) {}

        fn handle_event(&mut self, _event: &Event) -> bool {
            true
        }

        fn take_emitted_events(&mut self) -> Vec<Event> {
            Vec::new()
        }

        fn focused_text_input_state(&self) -> Option<TextInputElementState> {
            None
        }

        fn render(&mut self) -> Option<PreviewFrame<'_>> {
            None
        }
    }

    struct TestWindow {
        title: String,
        size: Size,
        scale_milli: u32,
    }

    impl TestWindow {
        fn new_for_host() -> Self {
            Self {
                title: String::from("Initial Preview"),
                size: Size::new(640.0, 480.0),
                scale_milli: 2000,
            }
        }
    }

    impl PlatformWindow for TestWindow {
        fn new(_app_id: &str, title: &str, size: Size) -> crate::Result<Self> {
            Ok(Self {
                title: title.to_string(),
                size,
                scale_milli: 1000,
            })
        }

        fn poll_event(&mut self) -> Option<Event> {
            None
        }

        fn output_scale_milli(&self) -> u32 {
            self.scale_milli
        }

        fn present(&mut self, _buffer: &Buffer) {}

        fn set_title(&mut self, title: &str) {
            self.title = title.to_string();
        }

        fn size(&self) -> Size {
            self.size
        }

        fn resize(&mut self, width: u32, height: u32) -> crate::Result<()> {
            self.size = Size::new(width as f32, height as f32);
            Ok(())
        }

        fn close(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn minimize(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn maximize(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn restore(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn request_move(&mut self) -> crate::Result<()> {
            Ok(())
        }

        fn create_popup(&mut self, _position: Point, _size: Size) -> crate::Result<u32> {
            Ok(0)
        }

        fn destroy_popup(&mut self, _surface_id: u32) -> crate::Result<()> {
            Ok(())
        }

        fn set_workarea(
            &mut self,
            _x: i32,
            _y: i32,
            _width: u32,
            _height: u32,
        ) -> crate::Result<()> {
            Ok(())
        }

        fn create_window_with_type(
            &mut self,
            _app_id: &str,
            title: &str,
            size: Size,
            _window_type: u32,
        ) -> crate::Result<Self> {
            Ok(Self {
                title: title.to_string(),
                size,
                scale_milli: 1000,
            })
        }

        fn move_window(&mut self, _x: i32, _y: i32) -> crate::Result<()> {
            Ok(())
        }

        fn set_window_type(&mut self, _surface_id: u32, _window_type: u32) -> crate::Result<()> {
            Ok(())
        }

        fn get_screen_size(&mut self) -> crate::Result<(u32, u32)> {
            Ok((1024, 768))
        }

        fn surface_id(&self) -> u32 {
            1
        }

        fn as_any_mut(&mut self) -> &mut dyn core::any::Any {
            self
        }

        fn set_resizable(&mut self, _resizable: bool) -> crate::Result<()> {
            Ok(())
        }

        fn set_opaque(&mut self, _opaque: bool) -> crate::Result<()> {
            Ok(())
        }

        fn set_menu_titles(&mut self, _menu_titles: &str) -> crate::Result<()> {
            Ok(())
        }
    }

    fn loaded_test_library() -> LoadedPreviewLibrary {
        LoadedPreviewLibrary {
            api: Box::new(TestLibrary::new()),
            _library: libloading::os::unix::Library::this().into(),
        }
    }

    fn host_with_loaded_library() -> PreviewHost {
        PreviewHost {
            session: Some(Box::new(TestSession {
                title: String::from("Initial Preview"),
            })),
            loaded: Some(loaded_test_library()),
            window: Box::new(TestWindow::new_for_host()),
            preview_id: PreviewId::new("button_preview"),
            scale_override_milli: None,
            sync_after_reload: false,
            full_present_frames: 0,
            gpu_present: None,
        }
    }

    fn host_without_loaded_library() -> PreviewHost {
        PreviewHost {
            session: Some(Box::new(TestSession {
                title: String::from("Initial Preview"),
            })),
            loaded: None,
            window: Box::new(TestWindow::new_for_host()),
            preview_id: PreviewId::new("button_preview"),
            scale_override_milli: None,
            sync_after_reload: false,
            full_present_frames: 0,
            gpu_present: None,
        }
    }

    #[test]
    fn switch_preview_by_exact_id_succeeds_and_updates_preview_id() {
        let mut host = host_with_loaded_library();

        host.switch_preview("counter_preview").unwrap();

        assert_eq!(host.preview_id.as_str(), "counter_preview");
        assert!(host.sync_after_reload);
        assert_eq!(host.full_present_frames, 2);
    }

    #[test]
    fn switch_preview_by_exact_name_succeeds_and_updates_preview_id() {
        let mut host = host_with_loaded_library();

        host.switch_preview("Counter Preview").unwrap();

        assert_eq!(host.preview_id.as_str(), "counter_preview");
        assert!(host.sync_after_reload);
        assert_eq!(host.full_present_frames, 2);
    }

    #[test]
    fn switch_preview_unknown_substring_returns_err() {
        let mut host = host_with_loaded_library();

        let error = host.switch_preview("Counter").unwrap_err();

        assert_eq!(error, "preview not found: Counter");
        assert_eq!(host.preview_id.as_str(), "button_preview");
    }

    #[test]
    fn switch_preview_without_loaded_library_returns_err() {
        let mut host = host_without_loaded_library();

        let error = host.switch_preview("counter_preview").unwrap_err();

        assert_eq!(error, "preview host has no loaded library");
        assert_eq!(host.preview_id.as_str(), "button_preview");
    }
}

impl Drop for PreviewHost {
    fn drop(&mut self) {
        self.session = None;
        self.loaded = None;
    }
}

fn error_to_string(error: Error) -> String {
    error.to_string()
}
