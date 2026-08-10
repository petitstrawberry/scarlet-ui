//! Two-slot shared-image backend and ScarletUI paint-backend integration.

use alloc::rc::Rc;
use alloc::vec::Vec;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::error::Error as UiError;
use scarlet_ui_core::geometry::{Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, PaintBackend, PaintContext};
use sgfx::{Context, Device, Image, Queue};

use crate::error::{Error, Result, Stage};
use crate::geometry::PixelBounds;
use crate::lowering::RenderSession;
use crate::sink::{
    SgfxBufferIdentity, SgfxCommitToken, SgfxFrameSink, SgfxSinkError, SgfxSinkStatus,
};

/// Default Scarlet graphics device used by the UI renderer.
pub const DEFAULT_GPU_DEVICE: &str = "/dev/gpu0";

struct SlotState {
    registered: Option<SgfxBufferIdentity>,
    retained: Option<SgfxCommitToken>,
    needs_full_commit: bool,
}

impl SlotState {
    fn new() -> Self {
        Self {
            registered: None,
            retained: None,
            needs_full_commit: true,
        }
    }
}

struct RetiredImage {
    identity: Option<SgfxBufferIdentity>,
    retained: Option<SgfxCommitToken>,
    image: Rc<Image>,
}

/// Native SGFX implementation of ScarletUI's backend-neutral paint contract.
///
/// The backend owns a persistent SGFX IR cache for each allocation generation
/// and alternates between exactly two shared images. A slot is never rendered
/// while SWS retains its complete `(window, buffer, generation, compositor
/// epoch)` identity.
pub struct SgfxPaintBackend<S> {
    sink: S,
    // Rust drops fields in declaration order. Keep every object owned by the
    // SGFX context ahead of the queue and the context itself so closing and
    // reopening a window cannot tear down the context before its images.
    session: Option<RenderSession>,
    slots: Vec<SlotState>,
    retired: Vec<RetiredImage>,
    queue: Queue,
    context: Context,
    scale_milli: u32,
    physical_width: u32,
    physical_height: u32,
    generation: u32,
    compositor_epoch: Option<u32>,
    next_slot: usize,
    front_slot: Option<usize>,
    supports_depth: bool,
}

impl<S: SgfxFrameSink> SgfxPaintBackend<S> {
    /// Open the default GPU and create a native SGFX paint backend.
    ///
    /// The graphics device, persistent IR resources, and both shared images
    /// are created and registered before this function succeeds. This keeps
    /// `SCARLET_UI_BACKEND=auto` fallback confined to initialization instead of
    /// surfacing an import failure on the first frame.
    ///
    /// # Arguments
    ///
    /// * `sink` - SWS frame sink sharing the window's accepted connection.
    /// * `size` - Initial logical target size.
    /// * `scale_milli` - Initial physical output scale.
    ///
    /// # Returns
    ///
    /// A ready backend, or a graphics-device initialization error.
    pub fn new(sink: S, size: Size, scale_milli: u32) -> Result<Self> {
        Self::with_device_path(sink, size, scale_milli, DEFAULT_GPU_DEVICE)
    }

    /// Open an explicit GPU device and create a native SGFX paint backend.
    ///
    /// # Arguments
    ///
    /// * `sink` - SWS frame sink sharing the window's accepted connection.
    /// * `size` - Initial logical target size.
    /// * `scale_milli` - Initial physical output scale.
    /// * `device_path` - Scarlet graphics device path.
    ///
    /// # Returns
    ///
    /// A ready backend, or a graphics-device initialization error.
    pub fn with_device_path(
        sink: S,
        size: Size,
        scale_milli: u32,
        device_path: &str,
    ) -> Result<Self> {
        let device = Device::open(device_path).map_err(|_| Error::sgfx(Stage::OpenDevice))?;
        let capabilities = device.capabilities();
        if !capabilities.supports_rendering() || !capabilities.supports_presentation() {
            return Err(Error::sgfx(Stage::OpenDevice));
        }
        let context = device
            .create_context()
            .map_err(|_| Error::sgfx(Stage::CreateContext))?;
        let queue = context
            .create_queue()
            .map_err(|_| Error::sgfx(Stage::CreateQueue))?;
        let (physical_width, physical_height) = physical_dimensions(size, scale_milli)?;
        let mut backend = Self {
            sink,
            context,
            queue,
            session: None,
            slots: alloc::vec![SlotState::new(), SlotState::new()],
            retired: Vec::new(),
            scale_milli: scale_milli.max(1),
            physical_width,
            physical_height,
            generation: 1,
            compositor_epoch: None,
            next_slot: 0,
            front_slot: None,
            supports_depth: capabilities.supports_depth(),
        };
        backend.initialize_shared_images()?;
        Ok(backend)
    }

    /// Borrow the platform frame sink.
    ///
    /// # Returns
    ///
    /// Shared reference to the sink.
    pub const fn sink(&self) -> &S {
        &self.sink
    }

    /// Mutably borrow the platform frame sink.
    ///
    /// # Returns
    ///
    /// Mutable reference to the sink.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume the renderer and return its platform frame sink.
    ///
    /// The final front buffer is not synchronously waited here; SWS window
    /// teardown owns that final cleanup and therefore cannot deadlock waiting
    /// for a replacement commit.
    ///
    /// # Returns
    ///
    /// The owned frame sink.
    pub fn into_sink(self) -> S {
        self.sink
    }

    fn initialize_shared_images(&mut self) -> Result<()> {
        self.refresh_compositor_epoch()?;
        self.ensure_session()?;

        for slot_index in 0..self.slots.len() {
            let identity = self.identity(slot_index)?;
            let registration = {
                let image = self
                    .session
                    .as_ref()
                    .and_then(|session| session.image(slot_index))
                    .ok_or(Error::InvalidFrame)?;
                self.sink.register_shared_image(identity, image)
            };
            if let Err(source) = registration {
                for state in &mut self.slots {
                    if let Some(registered) = state.registered.take() {
                        let _ = self.sink.destroy_shared_image(registered);
                    }
                }
                return Err(Error::sink(Stage::RegisterImage, source));
            }
            self.slots[slot_index].registered = Some(identity);
        }
        Ok(())
    }

    /// Render and atomically commit one frame.
    ///
    /// # Arguments
    ///
    /// * `paint` - Backend-neutral paint list and borrowed source buffers.
    /// * `background` - Straight-alpha clear color.
    /// * `physical_damage` - Physical presentation damage, or `None` for full.
    ///
    /// # Returns
    ///
    /// Success after synchronous SGFX submission and SWS commit. An empty
    /// damage slice is an idle frame and does not issue a protocol request.
    pub fn render_and_commit(
        &mut self,
        paint: &PaintContext<'_>,
        background: Color,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<()> {
        let render_areas = self.render_areas(physical_damage)?;
        if render_areas.is_empty() {
            return Ok(());
        }
        self.refresh_compositor_epoch()?;
        self.ensure_session()?;

        let slot_index = self.next_slot;
        let identity = self.identity(slot_index)?;
        if let Some(retained) = self.slots[slot_index].retained {
            self.sink
                .wait_until_released(retained)
                .map_err(|source| Error::sink(Stage::WaitForRelease, source))?;
            self.slots[slot_index].retained = None;
        }

        let copy_from = if physical_damage.is_some() {
            match self.front_slot {
                // A partial PaintContext contains only commands intersecting
                // the current damage. Seed the target with the complete
                // current front image before clearing and replaying those
                // commands, so alternating slots never depend on skipped
                // commands from an older frame.
                Some(front_slot) if front_slot != slot_index => Some(front_slot),
                Some(_) => return Err(Error::InvalidFrame),
                None if render_areas.len() == 1 && render_areas[0] == self.full_bounds() => None,
                None => return Err(Error::InvalidFrame),
            }
        } else {
            None
        };
        {
            let session = self.session.as_mut().ok_or(Error::InvalidFrame)?;
            session.render(
                &self.context,
                &self.queue,
                slot_index,
                copy_from,
                paint,
                background,
                self.scale_milli,
                &render_areas,
            )?;
        }

        if self.slots[slot_index].registered != Some(identity) {
            let image = self
                .session
                .as_ref()
                .and_then(|session| session.image(slot_index))
                .ok_or(Error::InvalidFrame)?;
            self.sink
                .register_shared_image(identity, image)
                .map_err(|source| Error::sink(Stage::RegisterImage, source))?;
            self.slots[slot_index].registered = Some(identity);
        }

        let commit_damage = self.commit_damage(slot_index, physical_damage)?;
        let retained = self
            .sink
            .commit_shared_image(identity, &commit_damage)
            .map_err(|source| Error::sink(Stage::CommitImage, source))?;

        self.slots[slot_index].retained = Some(retained);
        self.slots[slot_index].needs_full_commit = false;
        self.front_slot = Some(slot_index);
        self.next_slot = (slot_index + 1) % 2;
        self.cleanup_retired()?;
        Ok(())
    }

    fn refresh_compositor_epoch(&mut self) -> Result<()> {
        let status = self
            .sink
            .status()
            .map_err(|source| Error::sink(Stage::RegisterImage, source))?;
        let epoch = match status {
            SgfxSinkStatus::Ready { compositor_epoch } => compositor_epoch,
            SgfxSinkStatus::BackendLost { compositor_epoch } => {
                return Err(Error::sink(
                    Stage::RegisterImage,
                    SgfxSinkError::BackendLost { compositor_epoch },
                ));
            }
        };
        if epoch == 0 {
            return Err(Error::sink(
                Stage::RegisterImage,
                SgfxSinkError::InvalidIdentity,
            ));
        }
        if self.compositor_epoch == Some(epoch) {
            return Ok(());
        }
        self.compositor_epoch = Some(epoch);
        for slot in &mut self.slots {
            slot.registered = None;
            slot.retained = None;
            slot.needs_full_commit = true;
        }
        self.front_slot = None;
        self.retired.clear();
        Ok(())
    }

    fn ensure_session(&mut self) -> Result<()> {
        let session_matches = self.session.as_ref().is_some_and(|session| {
            session.image(0).is_some_and(|image| {
                image.width() == self.physical_width && image.height() == self.physical_height
            })
        });
        if session_matches {
            return Ok(());
        }

        let next_generation = if self.session.is_some() {
            self.generation
                .checked_add(1)
                .ok_or(Error::GenerationExhausted)?
        } else {
            self.generation
        };
        let replacement = RenderSession::new(
            &self.context,
            self.physical_width,
            self.physical_height,
            self.supports_depth,
        )?;
        if let Some(previous) = self.session.replace(replacement) {
            let images = previous.into_images();
            for (index, image) in images.into_iter().enumerate() {
                let state = self.slots.get(index).ok_or(Error::InvalidFrame)?;
                self.retired.push(RetiredImage {
                    identity: state.registered,
                    retained: state.retained,
                    image,
                });
            }
        }
        self.generation = next_generation;
        self.slots.clear();
        self.slots.push(SlotState::new());
        self.slots.push(SlotState::new());
        self.next_slot = 0;
        self.front_slot = None;
        Ok(())
    }

    fn identity(&self, slot: usize) -> Result<SgfxBufferIdentity> {
        let buffer_id = u32::try_from(slot)
            .ok()
            .and_then(|slot| slot.checked_add(1))
            .ok_or(Error::InvalidFrame)?;
        Ok(SgfxBufferIdentity {
            window_id: self.sink.window_id(),
            buffer_id,
            generation: self.generation,
            compositor_epoch: self.compositor_epoch.ok_or(Error::InvalidFrame)?,
        })
    }

    fn render_areas(&self, physical_damage: Option<&[DamageRect]>) -> Result<Vec<PixelBounds>> {
        let Some(damage) = physical_damage else {
            return Ok(alloc::vec![self.full_bounds()]);
        };
        let mut areas = Vec::new();
        areas
            .try_reserve(damage.len())
            .map_err(|_| Error::FrameTooComplex)?;
        for rect in damage {
            if let Some((x, y, width, height)) =
                clamp_damage(*rect, self.physical_width, self.physical_height)
            {
                areas.push(PixelBounds {
                    x,
                    y,
                    width,
                    height,
                });
            }
        }
        Ok(areas)
    }

    fn commit_damage(
        &self,
        slot: usize,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<Vec<DamageRect>> {
        let state = self.slots.get(slot).ok_or(Error::InvalidFrame)?;
        if state.needs_full_commit || physical_damage.is_none() {
            return Ok(alloc::vec![self.full_damage()]);
        }
        let mut damage = Vec::new();
        for rect in physical_damage.ok_or(Error::InvalidFrame)? {
            if let Some(rect) = clamp_damage(*rect, self.physical_width, self.physical_height) {
                damage.try_reserve(1).map_err(|_| Error::FrameTooComplex)?;
                damage.push(rect);
            }
        }
        if damage.is_empty() {
            return Err(Error::InvalidFrame);
        }
        Ok(damage)
    }

    fn cleanup_retired(&mut self) -> Result<()> {
        while let Some(mut retired) = self.retired.pop() {
            if let Some(identity) = retired.identity {
                if let Some(retained) = retired.retained {
                    match self.sink.wait_until_released(retained) {
                        Ok(()) => retired.retained = None,
                        Err(SgfxSinkError::BackendLost { .. }) => retired.retained = None,
                        Err(source) => {
                            self.retired.push(retired);
                            return Err(Error::sink(Stage::WaitForRelease, source));
                        }
                    }
                }
                match self.sink.destroy_shared_image(identity) {
                    Ok(()) | Err(SgfxSinkError::BackendLost { .. }) => {}
                    Err(source) => {
                        self.retired.push(retired);
                        return Err(Error::sink(Stage::DestroyImage, source));
                    }
                }
            }
            if let Ok(image) = Rc::try_unwrap(retired.image) {
                self.context
                    .release_image(image)
                    .map_err(|_| Error::sgfx(Stage::ReleaseSharedImage))?;
            }
        }
        Ok(())
    }

    const fn full_bounds(&self) -> PixelBounds {
        PixelBounds {
            x: 0,
            y: 0,
            width: self.physical_width,
            height: self.physical_height,
        }
    }

    const fn full_damage(&self) -> DamageRect {
        (0, 0, self.physical_width, self.physical_height)
    }
}

impl<S: SgfxFrameSink> PaintBackend for SgfxPaintBackend<S> {
    fn resize(&mut self, size: Size, scale_milli: u32) {
        self.scale_milli = scale_milli.max(1);
        if let Ok((width, height)) = physical_dimensions(size, scale_milli) {
            self.physical_width = width;
            self.physical_height = height;
        }
    }

    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background_color: Color,
        _logical_damage: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> scarlet_ui_core::Result<BackendFrame<'a>> {
        self.render_and_commit(context, background_color, physical_damage)
            .map_err(|_| UiError::RenderError)?;
        Ok(BackendFrame::External)
    }
}

fn physical_dimensions(size: Size, scale_milli: u32) -> Result<(u32, u32)> {
    if !size.width.is_finite()
        || !size.height.is_finite()
        || size.width <= 0.0
        || size.height <= 0.0
    {
        return Err(Error::InvalidFrame);
    }
    let logical_width = size.width as u32;
    let logical_height = size.height as u32;
    if logical_width == 0 || logical_height == 0 {
        return Err(Error::InvalidFrame);
    }
    let scale = scale_milli.max(1) as u64;
    let width = (u64::from(logical_width)
        .saturating_mul(scale)
        .saturating_add(999)
        / 1000)
        .max(1);
    let height = (u64::from(logical_height)
        .saturating_mul(scale)
        .saturating_add(999)
        / 1000)
        .max(1);
    Ok((
        u32::try_from(width).map_err(|_| Error::InvalidFrame)?,
        u32::try_from(height).map_err(|_| Error::InvalidFrame)?,
    ))
}

fn clamp_damage(damage: DamageRect, frame_width: u32, frame_height: u32) -> Option<DamageRect> {
    let (x, y, width, height) = damage;
    if width == 0 || height == 0 || x >= frame_width || y >= frame_height {
        return None;
    }
    let right = x.saturating_add(width).min(frame_width);
    let bottom = y.saturating_add(height).min(frame_height);
    if right <= x || bottom <= y {
        None
    } else {
        Some((x, y, right - x, bottom - y))
    }
}
