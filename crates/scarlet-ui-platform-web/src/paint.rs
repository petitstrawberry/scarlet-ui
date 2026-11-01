use scarlet_ui_core::{
    Error, Result,
    color::Color,
    compositor::DamageRect,
    geometry::{Point, Rect, Size},
    renderer::{BackendFrame, PaintBackend, PaintContext},
};
use scarlet_ui_renderer_sgfx::SgfxPaintEncoder;
use sgfx_backend_wgpu::{MappedTargetSession, WindowContext};
use std::{cell::RefCell, rc::Rc};

pub(crate) struct WebPaint {
    session: MappedTargetSession,
    window: WindowContext,
    encoder: SgfxPaintEncoder,
    scale: u32,
    resize_failed: bool,
    menus: Rc<RefCell<crate::menu::WebMenu>>,
}
impl WebPaint {
    pub fn new(
        window: WindowContext,
        width: u32,
        height: u32,
        menus: Rc<RefCell<crate::menu::WebMenu>>,
    ) -> Result<Self> {
        let encoder = SgfxPaintEncoder::new(width.max(1), height.max(1), true)
            .map_err(|_| Error::RenderError)?;
        let target = encoder.target_texture(0).ok_or(Error::RenderError)?;
        let session = window
            .create_mapped_target_session(encoder.resource_table(), &[target])
            .map_err(|_| Error::RenderError)?;
        Ok(Self {
            session,
            window,
            encoder,
            scale: 1000,
            resize_failed: false,
            menus,
        })
    }
}
impl PaintBackend for WebPaint {
    fn resize(&mut self, size: Size, scale: u32) {
        self.scale = scale.max(1);
        let width = (size.width * self.scale as f32 / 1000.0).ceil().max(1.0) as u32;
        let height = ((size.height + self.menus.borrow().height()) * self.scale as f32 / 1000.0)
            .ceil()
            .max(1.0) as u32;
        self.resize_failed = false;
        if width == self.encoder.width() && height == self.encoder.height() {
            return;
        }
        self.resize_failed = true;
        // Allocate the new targets before replacing the old session.
        if let Ok(encoder) = SgfxPaintEncoder::new(width, height, true) {
            if let Some(target) = encoder.target_texture(0) {
                if let Ok(session) = self
                    .window
                    .create_mapped_target_session(encoder.resource_table(), &[target])
                {
                    self.window.resize(width, height);
                    self.session = session;
                    self.encoder = encoder;
                    self.resize_failed = false;
                }
            }
        }
    }
    fn render<'a>(
        &'a mut self,
        context: &PaintContext<'_>,
        background: Color,
        _: Option<&[Rect]>,
        physical_damage: Option<&[DamageRect]>,
    ) -> Result<BackendFrame<'a>> {
        if self.resize_failed {
            return Err(Error::RenderError);
        }
        // The retained UI paint list may contain only dirty widgets. Keep the
        // persistent offscreen target and clear/repaint only those damaged areas;
        // presentation blits the complete target into each fresh swapchain image.
        let menus = self.menus.borrow();
        let offset = menus.height();
        let shifted_damage = physical_damage.map(|damage| {
            let physical_offset = (offset * self.scale as f32 / 1000.0).round() as u32;
            damage
                .iter()
                .map(|&(x, y, w, h)| (x, y.saturating_add(physical_offset), w, h))
                .collect::<Vec<_>>()
        });
        let areas = crate::damage::render_areas(
            self.encoder.width(),
            self.encoder.height(),
            shifted_damage.as_deref(),
        );
        let mut composed = PaintContext::new();
        composed.append_at(context, Point::new(0.0, offset));
        menus.paint(&mut composed);
        if areas.is_empty() {
            return Err(Error::RenderError);
        }
        self.encoder
            .encode_frame(
                &mut self.session.executor(),
                0,
                None,
                &composed,
                background,
                self.scale,
                &areas,
            )
            .map_err(|_| Error::RenderError)?;
        self.window
            .present(
                &self.session,
                self.encoder.target_texture(0).ok_or(Error::RenderError)?,
            )
            .map_err(|_| Error::RenderError)?;
        Ok(BackendFrame::External)
    }
}
