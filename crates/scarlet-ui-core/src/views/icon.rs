//! Typed icon views backed by Tabler alpha-mask rendering.

use crate::color::{Color, ColorPalette};
use crate::element::{Element, ElementRenderObject, LayoutConstraints, RenderElement};
use crate::geometry::{Point, Rect, Size};
use crate::icon::{Icon, IconFill, IconSize, IconStyle, IconWeight};
use crate::renderer::PaintContext;
use crate::view::View;
use alloc::boxed::Box;
use alloc::string::String;
use core::any::Any;

/// Semantic kind used to choose a file-system icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileIconKind {
    /// A directory or folder.
    Folder,
    /// A JPEG, PNG, or other image file.
    Image,
    /// A document or text-like file.
    Document,
    /// An audio file.
    Audio,
    /// A video file.
    Video,
    /// An archive or compressed file.
    Archive,
    /// A file without a recognized semantic type.
    Unknown,
}

impl FileIconKind {
    /// Infer a semantic kind from a path's final extension.
    ///
    /// # Arguments
    ///
    /// * `path` - File path or display name to inspect.
    ///
    /// # Returns
    ///
    /// A best-effort file icon kind. Paths without an extension map to
    /// [`FileIconKind::Unknown`].
    pub fn from_path(path: &str) -> Self {
        let extension = path.rsplit_once('.').map(|(_, extension)| extension);
        let Some(extension) = extension else {
            return Self::Unknown;
        };

        if ["jpg", "jpeg", "png", "gif", "bmp", "webp"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Image
        } else if ["mp3", "wav", "ogg", "flac", "m4a"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Audio
        } else if ["mp4", "mkv", "webm", "mov", "avi"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Video
        } else if ["zip", "tar", "gz", "xz", "bz2", "7z", "rar"]
            .iter()
            .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Archive
        } else if [
            "txt", "md", "rs", "c", "h", "cpp", "toml", "json", "xml", "html", "pdf",
        ]
        .iter()
        .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        {
            Self::Document
        } else {
            Self::Unknown
        }
    }

    fn icon(self) -> Icon {
        match self {
            Self::Folder => Icon::Folder,
            Self::Image => Icon::Photo,
            Self::Document => Icon::FileText,
            Self::Audio => Icon::Music,
            Self::Video => Icon::Video,
            Self::Archive => Icon::Archive,
            Self::Unknown => Icon::File,
        }
    }

    fn color(self, palette: ColorPalette) -> Color {
        match self {
            Self::Folder => palette.warning(),
            Self::Image => palette.success(),
            Self::Audio => palette.info(),
            Self::Video => palette.primary(),
            Self::Archive => palette.secondary(),
            Self::Document | Self::Unknown => palette.text_secondary(),
        }
    }
}

/// A general-purpose typed icon view.
#[derive(Clone)]
pub struct IconView {
    icon: Icon,
    size: IconSize,
    style: IconStyle,
    color: Option<Color>,
}

impl IconView {
    /// Create an icon view.
    ///
    /// # Arguments
    ///
    /// * `icon` - Typed Tabler icon to render.
    ///
    /// # Returns
    ///
    /// A medium, theme-colored outline icon.
    pub fn new(icon: Icon) -> Self {
        Self {
            icon,
            size: IconSize::default(),
            style: IconStyle::default(),
            color: None,
        }
    }

    /// Set a standard or explicit logical size.
    ///
    /// # Arguments
    ///
    /// * `size` - Icon size.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    /// Set the outline rendering style.
    ///
    /// # Arguments
    ///
    /// * `style` - Stroke style used during alpha-mask rasterization.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn style(mut self, style: IconStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the outline stroke width in Tabler view-box units.
    ///
    /// # Arguments
    ///
    /// * `width` - Stroke width, normally between 1.0 and 2.0.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.style = self.style.stroke_width(width);
        self
    }

    /// Set a semantic outline weight.
    ///
    /// # Arguments
    ///
    /// * `weight` - Thin, normal, or bold stroke weight.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn weight(mut self, weight: IconWeight) -> Self {
        self.style = self.style.weight(weight);
        self
    }

    /// Select the outline or filled vector treatment.
    ///
    /// # Arguments
    ///
    /// * `fill` - Requested vector treatment.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn fill(mut self, fill: IconFill) -> Self {
        self.style = self.style.fill(fill);
        self
    }

    /// Use the official Tabler filled variant when available.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn filled(self) -> Self {
        self.fill(IconFill::Filled)
    }

    /// Override the theme foreground color.
    ///
    /// # Arguments
    ///
    /// * `color` - Explicit icon tint.
    ///
    /// # Returns
    ///
    /// The updated icon view.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Return the represented icon.
    ///
    /// # Returns
    ///
    /// The typed icon value.
    pub fn icon(&self) -> Icon {
        self.icon
    }
}

/// A typed icon view selected from file-system semantics.
#[derive(Clone)]
pub struct FileIconView {
    kind: FileIconKind,
    size: IconSize,
    style: IconStyle,
    color: Option<Color>,
}

impl FileIconView {
    /// Create a file icon view.
    ///
    /// # Arguments
    ///
    /// * `kind` - Semantic file type.
    ///
    /// # Returns
    ///
    /// A large file icon view.
    pub fn new(kind: FileIconKind) -> Self {
        Self {
            kind,
            size: IconSize::ExtraLarge,
            style: IconStyle::default(),
            color: None,
        }
    }

    /// Infer a file icon from a path.
    ///
    /// # Arguments
    ///
    /// * `path` - File path or display name.
    ///
    /// # Returns
    ///
    /// A file icon view using the inferred semantic type.
    pub fn from_path(path: impl Into<String>) -> Self {
        let path = path.into();
        Self::new(FileIconKind::from_path(&path))
    }

    /// Set a standard or explicit logical size.
    ///
    /// # Arguments
    ///
    /// * `size` - Icon size.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }

    /// Set the outline rendering style.
    ///
    /// # Arguments
    ///
    /// * `style` - Stroke style used during alpha-mask rasterization.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn style(mut self, style: IconStyle) -> Self {
        self.style = style;
        self
    }

    /// Set the outline stroke width in Tabler view-box units.
    ///
    /// # Arguments
    ///
    /// * `width` - Stroke width, normally between 1.0 and 2.0.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn stroke_width(mut self, width: f32) -> Self {
        self.style = self.style.stroke_width(width);
        self
    }

    /// Set a semantic outline weight.
    ///
    /// # Arguments
    ///
    /// * `weight` - Thin, normal, or bold stroke weight.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn weight(mut self, weight: IconWeight) -> Self {
        self.style = self.style.weight(weight);
        self
    }

    /// Select the outline or filled vector treatment.
    ///
    /// # Arguments
    ///
    /// * `fill` - Requested vector treatment.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn fill(mut self, fill: IconFill) -> Self {
        self.style = self.style.fill(fill);
        self
    }

    /// Use the official Tabler filled variant when available.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn filled(self) -> Self {
        self.fill(IconFill::Filled)
    }

    /// Override the semantic file color.
    ///
    /// # Arguments
    ///
    /// * `color` - Explicit icon tint.
    ///
    /// # Returns
    ///
    /// The updated file icon view.
    pub fn color(mut self, color: Color) -> Self {
        self.color = Some(color);
        self
    }

    /// Return the semantic file kind.
    ///
    /// # Returns
    ///
    /// The file icon kind.
    pub fn kind(&self) -> FileIconKind {
        self.kind
    }
}

#[derive(Clone, Copy)]
struct IconRenderObject {
    icon: Icon,
    preferred_size: IconSize,
    style: IconStyle,
    color: Option<Color>,
    size: Size,
}

impl IconRenderObject {
    fn new(icon: Icon, preferred_size: IconSize, style: IconStyle, color: Option<Color>) -> Self {
        Self {
            icon,
            preferred_size,
            style,
            color,
            size: Size::ZERO,
        }
    }
}

pub(crate) fn paint_icon(
    ctx: &mut PaintContext,
    origin: Point,
    size: f32,
    icon: Icon,
    style: IconStyle,
    color: Color,
) {
    ctx.draw_icon(
        Rect::from_xywh(origin.x, origin.y, size, size),
        icon,
        style,
        color,
    );
}

impl ElementRenderObject for IconRenderObject {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        let preferred = self.preferred_size.logical_pixels() as f32;
        let maximum_width = if constraints.max_width.is_finite() {
            constraints.max_width
        } else {
            preferred
        };
        let maximum_height = if constraints.max_height.is_finite() {
            constraints.max_height
        } else {
            preferred
        };
        let side = preferred.min(maximum_width).min(maximum_height).max(1.0);
        self.size = Size::new(
            side.max(constraints.min_width),
            side.max(constraints.min_height),
        );
        self.size
    }

    fn size(&self) -> Size {
        self.size
    }

    fn render(&mut self) {}

    fn paint(&self, ctx: &mut PaintContext, origin: Point) -> bool {
        let side = self.size.width.min(self.size.height);
        let icon_origin = Point::new(
            origin.x + (self.size.width - side) * 0.5,
            origin.y + (self.size.height - side) * 0.5,
        );
        paint_icon(
            ctx,
            icon_origin,
            side,
            self.icon,
            self.style,
            self.color.unwrap_or_else(|| ColorPalette::default().text()),
        );
        true
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

impl View for IconView {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::new(
            self.clone(),
            IconRenderObject::new(self.icon, self.size, self.style, self.color),
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

impl View for FileIconView {
    fn create_element(&self) -> Box<dyn Element> {
        let palette = ColorPalette::default();
        Box::new(RenderElement::new(
            self.clone(),
            IconRenderObject::new(
                self.kind.icon(),
                self.size,
                self.style,
                Some(self.color.unwrap_or_else(|| self.kind.color(palette))),
            ),
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
