//! Shared-image lifecycle and ScarletUI paint-backend integration.

use alloc::vec::Vec;
use core::fmt;

use scarlet_ui_core::color::Color;
use scarlet_ui_core::compositor::DamageRect;
use scarlet_ui_core::geometry::{Rect, Size};
use scarlet_ui_core::renderer::{BackendFrame, PaintBackend, PaintContext};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx::{BackendKind, Context, Device, MappedTargetSession};

use crate::{SgfxBufferIdentity, SgfxCommitToken, SgfxFrameSink, SgfxSinkError, SgfxSinkStatus};

/// Default Scarlet graphics device used by the UI platform integration.
pub const DEFAULT_GPU_DEVICE: &str = "/dev/gpu0";

// SWS retains the last presented client image until a replacement reaches the
// compositor. Two targets therefore serialize rendering behind that release
// and can turn a narrowly missed 60 Hz deadline into a stable 30 Hz cadence.
// A third target keeps one image presented, one pending, and one renderable.
const PRESENTATION_SLOT_COUNT: usize = 3;

/// SGFX/SWS integration operation that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// Opening the graphics device.
    OpenDevice,
    /// Creating the graphics context.
    CreateContext,
    /// Creating or mapping the platform target session.
    CreateSession,
    /// Encoding or executing the portable command stream.
    Render,
    /// Registering a shared image with SWS.
    RegisterImage,
    /// Waiting for an SWS-retained image.
    WaitForRelease,
    /// Committing an image to SWS.
    CommitImage,
    /// Destroying a retired image registration.
    DestroyImage,
}

/// Error returned by the SWS SGFX paint backend.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// An SGFX facade operation failed.
    Sgfx(Stage),
    /// A portable ScarletUI encoding or execution operation failed.
    Render,
    /// An SWS sink operation failed.
    Sink { stage: Stage, source: SgfxSinkError },
    /// A frame dimension or lifecycle transition was invalid.
    InvalidFrame,
    /// The allocation-generation counter was exhausted.
    GenerationExhausted,
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sgfx(stage) => write!(formatter, "SGFX operation failed at {stage:?}"),
            Self::Render => formatter.write_str("ScarletUI SGFX rendering failed"),
            Self::Sink { stage, source } => {
                write!(formatter, "SGFX sink failed at {stage:?}: {source}")
            }
            Self::InvalidFrame => formatter.write_str("invalid ScarletUI frame"),
            Self::GenerationExhausted => formatter.write_str("SGFX buffer generation exhausted"),
        }
    }
}

/// Result returned by the SWS SGFX paint backend.
pub type Result<T> = core::result::Result<T, Error>;

#[derive(Clone, Copy)]
struct SlotState {
    registered: Option<SgfxBufferIdentity>,
    retained: Option<SgfxCommitToken>,
    needs_full_commit: bool,
}

impl SlotState {
    const fn new() -> Self {
        Self {
            registered: None,
            retained: None,
            needs_full_commit: true,
        }
    }
}

struct RetiredGeneration {
    // The SGFX-owned session retains its physical images and cache together.
    // It must outlive every SWS registration and retained use below.
    session: MappedTargetSession,
    identities: [Option<SgfxBufferIdentity>; PRESENTATION_SLOT_COUNT],
    retained: [Option<SgfxCommitToken>; PRESENTATION_SLOT_COUNT],
}

/// Native SGFX implementation of ScarletUI's backend-neutral paint contract.
///
/// This platform object owns SWS presentation policy and alternates between
/// exactly two images. The physical image/resource/context/queue association
/// is owned by [`MappedTargetSession`], and command execution is delegated to
/// its backend-owned executor.
pub struct SgfxPaintBackend<S> {
    sink: S,
    // Rust drops fields in declaration order. Keep every session that owns
    // physical resources ahead of the context that created it.
    encoder: Option<SgfxPaintEncoder>,
    session: Option<MappedTargetSession>,
    retired: Vec<RetiredGeneration>,
    context: Context,
    slots: [SlotState; PRESENTATION_SLOT_COUNT],
    scale_milli: u32,
    physical_width: u32,
    physical_height: u32,
    generation: u32,
    compositor_epoch: Option<u32>,
    next_slot: usize,
    front_slot: Option<usize>,
    supports_depth: bool,
    backend_kind: BackendKind,
}

impl<S: SgfxFrameSink> SgfxPaintBackend<S> {
    /// Open the default GPU and initialize the shared presentation images.
    ///
    /// # Arguments
    ///
    /// * `sink` - SWS frame sink sharing the window's accepted connection.
    /// * `size` - Initial logical target size.
    /// * `scale_milli` - Initial physical output scale.
    ///
    /// # Returns
    ///
    /// A ready backend, or an SGFX/session/SWS initialization error.
    pub fn new(sink: S, size: Size, scale_milli: u32) -> Result<Self> {
        Self::with_device_path(sink, size, scale_milli, DEFAULT_GPU_DEVICE)
    }

    /// Open an explicit GPU device and initialize the shared images.
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
    /// A ready backend, or an SGFX/session/SWS initialization error.
    pub fn with_device_path(
        sink: S,
        size: Size,
        scale_milli: u32,
        device_path: &str,
    ) -> Result<Self> {
        let device = Device::open(device_path).map_err(|_| Error::Sgfx(Stage::OpenDevice))?;
        let capabilities = device.capabilities();
        let backend_kind = device.backend();
        if !capabilities.supports_rendering() || !capabilities.supports_presentation() {
            return Err(Error::Sgfx(Stage::OpenDevice));
        }
        let context = device
            .create_context()
            .map_err(|_| Error::Sgfx(Stage::CreateContext))?;
        let (physical_width, physical_height) = physical_dimensions(size, scale_milli)?;
        let mut backend = Self {
            sink,
            encoder: None,
            session: None,
            retired: Vec::new(),
            context,
            slots: [SlotState::new(); PRESENTATION_SLOT_COUNT],
            scale_milli: scale_milli.max(1),
            physical_width,
            physical_height,
            generation: 1,
            compositor_epoch: None,
            next_slot: 0,
            front_slot: None,
            supports_depth: capabilities.supports_depth(),
            backend_kind,
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

    /// Return the complete SGFX backend selected for this platform renderer.
    ///
    /// # Returns
    ///
    /// Stable backend identity selected by the SGFX frontend.
    pub const fn backend_kind(&self) -> BackendKind {
        self.backend_kind
    }

    /// Mutably borrow the platform frame sink.
    ///
    /// # Returns
    ///
    /// Mutable reference to the sink.
    pub fn sink_mut(&mut self) -> &mut S {
        &mut self.sink
    }

    /// Consume the backend and return its platform frame sink.
    ///
    /// The final front image is not synchronously waited here; SWS window
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
        for slot in 0..PRESENTATION_SLOT_COUNT {
            let identity = self.identity(slot)?;
            let target = self
                .encoder
                .as_ref()
                .and_then(|encoder| encoder.target_texture(slot))
                .ok_or(Error::InvalidFrame)?;
            let image = self
                .session
                .as_ref()
                .ok_or(Error::InvalidFrame)?
                .image(target)
                .map_err(|_| Error::Sgfx(Stage::CreateSession))?;
            if let Err(source) = self.sink.register_shared_image(identity, image) {
                for state in &mut self.slots {
                    if let Some(registered) = state.registered.take() {
                        let _ = self.sink.destroy_shared_image(registered);
                    }
                }
                return Err(Error::Sink {
                    stage: Stage::RegisterImage,
                    source,
                });
            }
            self.slots[slot].registered = Some(identity);
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
    /// Success after synchronous SGFX execution and SWS commit. An empty
    /// damage slice is an idle frame and issues no protocol request.
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

        let slot = self.next_slot;
        let identity = self.identity(slot)?;
        if let Some(retained) = self.slots[slot].retained {
            self.sink
                .wait_until_released(retained)
                .map_err(|source| Error::Sink {
                    stage: Stage::WaitForRelease,
                    source,
                })?;
            self.slots[slot].retained = None;
        }
        let copy_from = if physical_damage.is_some() {
            match self.front_slot {
                Some(front) if front != slot => Some(front),
                Some(_) => return Err(Error::InvalidFrame),
                None if render_areas == [self.full_bounds()] => None,
                None => return Err(Error::InvalidFrame),
            }
        } else {
            None
        };

        {
            let encoder = self.encoder.as_mut().ok_or(Error::InvalidFrame)?;
            let session = self.session.as_mut().ok_or(Error::InvalidFrame)?;
            let mut executor = session.executor();
            encoder
                .encode_frame(
                    &mut executor,
                    slot,
                    copy_from,
                    paint,
                    background,
                    self.scale_milli,
                    &render_areas,
                )
                .map_err(|_| Error::Render)?;
        }

        if self.slots[slot].registered != Some(identity) {
            let target = self
                .encoder
                .as_ref()
                .and_then(|encoder| encoder.target_texture(slot))
                .ok_or(Error::InvalidFrame)?;
            let image = self
                .session
                .as_ref()
                .ok_or(Error::InvalidFrame)?
                .image(target)
                .map_err(|_| Error::Sgfx(Stage::CreateSession))?;
            self.sink
                .register_shared_image(identity, image)
                .map_err(|source| Error::Sink {
                    stage: Stage::RegisterImage,
                    source,
                })?;
            self.slots[slot].registered = Some(identity);
        }
        let damage = self.commit_damage(slot, physical_damage)?;
        let retained = self
            .sink
            .commit_shared_image(identity, &damage)
            .map_err(|source| Error::Sink {
                stage: Stage::CommitImage,
                source,
            })?;
        self.slots[slot].retained = Some(retained);
        self.slots[slot].needs_full_commit = false;
        self.front_slot = Some(slot);
        self.next_slot = (slot + 1) % PRESENTATION_SLOT_COUNT;
        self.cleanup_retired()?;
        Ok(())
    }

    fn refresh_compositor_epoch(&mut self) -> Result<()> {
        let status = self.sink.status().map_err(|source| Error::Sink {
            stage: Stage::RegisterImage,
            source,
        })?;
        let epoch = match status {
            SgfxSinkStatus::Ready { compositor_epoch } => compositor_epoch,
            SgfxSinkStatus::BackendLost { compositor_epoch } => {
                return Err(Error::Sink {
                    stage: Stage::RegisterImage,
                    source: SgfxSinkError::BackendLost { compositor_epoch },
                });
            }
        };
        if epoch == 0 {
            return Err(Error::Sink {
                stage: Stage::RegisterImage,
                source: SgfxSinkError::InvalidIdentity,
            });
        }
        if self.compositor_epoch == Some(epoch) {
            return Ok(());
        }
        self.compositor_epoch = Some(epoch);
        self.slots = [SlotState::new(); PRESENTATION_SLOT_COUNT];
        self.front_slot = None;
        // Backend loss invalidates every old registration and retained use.
        self.retired.clear();
        Ok(())
    }

    fn ensure_session(&mut self) -> Result<()> {
        let matches = self.encoder.as_ref().is_some_and(|encoder| {
            encoder.width() == self.physical_width && encoder.height() == self.physical_height
        });
        if matches && self.session.is_some() {
            return Ok(());
        }

        let replacing_session = self.session.is_some();
        let next_generation = if replacing_session {
            self.generation
                .checked_add(1)
                .ok_or(Error::GenerationExhausted)?
        } else {
            self.generation
        };

        // A successful SWS window resize is an explicit presentation
        // discontinuity: SWS switches the window back to its resized SHM
        // backing and releases every retained image from the old extent. Wait
        // for those releases, deregister the images, and drop the complete old
        // session before allocating replacements. Keeping the old three-image
        // session alive while materializing another three images creates a
        // roughly 50 MiB transient color-buffer peak at 1080p and can fail on
        // otherwise healthy low-memory systems.
        if let Some(previous) = self.session.take() {
            self.retired.push(RetiredGeneration {
                session: previous,
                identities: self.slots.map(|slot| slot.registered),
                retained: self.slots.map(|slot| slot.retained),
            });
            self.encoder = None;
            self.slots = [SlotState::new(); PRESENTATION_SLOT_COUNT];
            self.next_slot = 0;
            self.front_slot = None;
        }
        self.cleanup_retired()?;
        // Preserve the new identity generation even if materializing its
        // physical images fails. Retrying an allocation must not reuse an
        // identity that SWS has already observed for the retired extent.
        self.generation = next_generation;

        let encoder = SgfxPaintEncoder::with_target_count(
            self.physical_width,
            self.physical_height,
            self.supports_depth,
            PRESENTATION_SLOT_COUNT,
        )
        .map_err(|_| Error::Render)?;
        let mut targets = Vec::new();
        targets
            .try_reserve_exact(PRESENTATION_SLOT_COUNT)
            .map_err(|_| Error::InvalidFrame)?;
        for slot in 0..PRESENTATION_SLOT_COUNT {
            targets.push(encoder.target_texture(slot).ok_or(Error::InvalidFrame)?);
        }
        let session = self
            .context
            .create_mapped_target_session(encoder.resource_table(), &targets)
            .map_err(|_| Error::Sgfx(Stage::CreateSession))?;

        self.session = Some(session);
        self.encoder = Some(encoder);
        self.generation = next_generation;
        self.slots = [SlotState::new(); PRESENTATION_SLOT_COUNT];
        self.next_slot = 0;
        self.front_slot = None;
        Ok(())
    }

    fn identity(&self, slot: usize) -> Result<SgfxBufferIdentity> {
        let buffer_id = u32::try_from(slot)
            .ok()
            .and_then(|value| value.checked_add(1))
            .ok_or(Error::InvalidFrame)?;
        Ok(SgfxBufferIdentity {
            window_id: self.sink.window_id(),
            buffer_id,
            generation: self.generation,
            compositor_epoch: self.compositor_epoch.ok_or(Error::InvalidFrame)?,
        })
    }

    fn cleanup_retired(&mut self) -> Result<()> {
        while let Some(mut retired) = self.retired.pop() {
            for slot in 0..PRESENTATION_SLOT_COUNT {
                if let Some(identity) = retired.identities[slot] {
                    if let Some(retained) = retired.retained[slot] {
                        match self.sink.wait_until_released(retained) {
                            Ok(()) | Err(SgfxSinkError::BackendLost { .. }) => {
                                retired.retained[slot] = None;
                            }
                            Err(source) => {
                                self.retired.push(retired);
                                return Err(Error::Sink {
                                    stage: Stage::WaitForRelease,
                                    source,
                                });
                            }
                        }
                    }
                    match self.sink.destroy_shared_image(identity) {
                        Ok(()) | Err(SgfxSinkError::BackendLost { .. }) => {
                            retired.identities[slot] = None;
                        }
                        Err(source) => {
                            self.retired.push(retired);
                            return Err(Error::Sink {
                                stage: Stage::DestroyImage,
                                source,
                            });
                        }
                    }
                }
            }
            // Dropping the complete SGFX session after deregistration releases
            // its mapped images/cache before its context.
            drop(retired.session);
        }
        Ok(())
    }

    fn render_areas(&self, damage: Option<&[DamageRect]>) -> Result<Vec<DamageRect>> {
        let Some(damage) = damage else {
            return Ok(alloc::vec![self.full_bounds()]);
        };
        let mut areas = Vec::new();
        areas
            .try_reserve(damage.len())
            .map_err(|_| Error::InvalidFrame)?;
        for rect in damage {
            if let Some((x, y, width, height)) =
                clamp_damage(*rect, self.physical_width, self.physical_height)
            {
                areas.push((x, y, width, height));
            }
        }
        Ok(areas)
    }

    fn commit_damage(&self, slot: usize, damage: Option<&[DamageRect]>) -> Result<Vec<DamageRect>> {
        if self
            .slots
            .get(slot)
            .ok_or(Error::InvalidFrame)?
            .needs_full_commit
            || damage.is_none()
        {
            return Ok(alloc::vec![self.full_damage()]);
        }
        let mut result = Vec::new();
        for rect in damage.ok_or(Error::InvalidFrame)? {
            if let Some(rect) = clamp_damage(*rect, self.physical_width, self.physical_height) {
                result.try_reserve(1).map_err(|_| Error::InvalidFrame)?;
                result.push(rect);
            }
        }
        if result.is_empty() {
            Err(Error::InvalidFrame)
        } else {
            Ok(result)
        }
    }

    const fn full_bounds(&self) -> DamageRect {
        (0, 0, self.physical_width, self.physical_height)
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
            .map_err(|_| scarlet_ui_core::error::Error::RenderError)?;
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
    let width = size.width as u32;
    let height = size.height as u32;
    if width == 0 || height == 0 {
        return Err(Error::InvalidFrame);
    }
    let scale = u64::from(scale_milli.max(1));
    let width = (u64::from(width).saturating_mul(scale).saturating_add(999) / 1000).max(1);
    let height = (u64::from(height).saturating_mul(scale).saturating_add(999) / 1000).max(1);
    Ok((
        u32::try_from(width).map_err(|_| Error::InvalidFrame)?,
        u32::try_from(height).map_err(|_| Error::InvalidFrame)?,
    ))
}

fn clamp_damage(damage: DamageRect, width: u32, height: u32) -> Option<DamageRect> {
    let (x, y, rect_width, rect_height) = damage;
    if rect_width == 0 || rect_height == 0 || x >= width || y >= height {
        return None;
    }
    let right = x.saturating_add(rect_width).min(width);
    let bottom = y.saturating_add(rect_height).min(height);
    (right > x && bottom > y).then_some((x, y, right - x, bottom - y))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn physical_dimensions_round_up_fractional_scale() {
        assert_eq!(physical_dimensions(Size::new(3.0, 5.0), 1_500), Ok((5, 8)));
    }

    #[test]
    fn clamp_damage_rejects_empty_and_clips_to_target() {
        assert_eq!(clamp_damage((2, 3, 10, 10), 8, 9), Some((2, 3, 6, 6)));
        assert_eq!(clamp_damage((8, 0, 1, 1), 8, 9), None);
        assert_eq!(clamp_damage((0, 0, 0, 1), 8, 9), None);
    }

    #[test]
    fn new_slot_requires_full_commit_and_is_not_retained() {
        let slot = SlotState::new();
        assert!(slot.needs_full_commit);
        assert_eq!(slot.registered, None);
        assert_eq!(slot.retained, None);
    }
}
