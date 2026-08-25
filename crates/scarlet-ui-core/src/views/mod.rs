//! Views module - Built-in View implementations
//!
//! This module provides common View implementations like Window, Text, Button, etc.

mod button;
mod canvas;
pub mod containers;
mod divider;
mod either;
mod grid;
mod header;
mod icon;
mod image;
mod keyed;
mod lazy_vstack;
mod list;
pub mod menu;
pub mod modifiers;
pub mod navigation;
mod progress;
mod rectangle;
mod scroll;
mod select;
mod slider;
mod spacer;
mod split;
pub(crate) mod style;
mod surface;
mod tab;
mod text;
pub(crate) mod text_field;
mod text_grid;
pub(crate) mod text_grid_boxdraw;
pub mod text_view;
mod toggle;
mod window;

pub use button::{Button, ButtonCallback, ButtonRenderObject};
pub use canvas::{CanvasEventHandler, CanvasRenderCallback, CanvasRenderObject, CanvasView};
pub use divider::{Divider, DividerOrientation, DividerRenderObject};
pub use either::{Either, Either3, Either4, Either5, Either6};
pub use grid::GridView;
pub use header::{HeaderBar, HeaderBarRenderObject};
pub use icon::IconView;
pub use image::{
    BitmapImage, Image, ImageFit, ImageRenderObject, ImageSource, VectorImageData, VectorTriangle,
};
pub use keyed::Keyed;
pub use lazy_vstack::LazyVStack;
pub use list::ListView;
pub use menu::{Menu, MenuAction, MenuBar, MenuItem, MenuItemContent};
pub use navigation::{NavigationLink, NavigationView};
pub use progress::{ProgressView, ProgressViewRenderObject};
pub use rectangle::{Rectangle, RectangleRenderObject};
pub use scroll::{
    ScrollAxis, ScrollView, ScrollViewRenderObject, ScrollWheelDirection, ScrollbarVisibility,
};
pub use select::{Select, SelectChangeCallback, SelectRenderObject};
pub use slider::{Slider, SliderRenderObject};
pub use spacer::{Spacer, SpacerRenderObject};
pub use split::{SplitAxis, SplitView, SplitViewRenderObject};
pub use surface::{ElevationRole, Surface, SurfaceRole};
pub use tab::{TabItem, TabView, TabViewRenderObject};
pub use text::{Text, TextRenderObject};
pub use text_field::{TextField, TextFieldRenderObject};
pub use text_grid::{
    TextGrid, TextGridBuffer, TextGridCell, TextGridCursor, TextGridRenderObject,
    text_grid_cell_width,
};
pub use text_view::{
    EditDelta, TabMode, TextDocument, TextPosition, TextSelection, TextView, TextViewRenderObject,
    TextViewScroll, WrapMode,
};
pub use toggle::{Toggle, ToggleRenderObject};
pub use window::window_type;
pub use window::{
    Window, WindowContentLayout, WindowInfo, WindowRenderElement, WindowRenderObject,
};

// Re-export modifiers for convenience
pub use modifiers::{
    AlignmentFrame, AlignmentRenderObject, Background, BackgroundRenderObject, Border,
    BorderRenderObject, Focusable, FocusableRenderObject, Frame, FrameRenderObject, OnKey,
    OnKeyRenderObject, Padding, PaddingRenderObject, RepaintBoundary, RepaintBoundaryRenderObject,
    SetSize, SizeRenderObject,
};

// Re-export containers for convenience
pub use containers::{HStack, VStack, ZStack, ZStackRenderObject};
