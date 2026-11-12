//! View Modifiers - SwiftUI-style view modifiers
//!
//! This module provides view modifiers that wrap and transform child views.

mod alignment;
mod animation_scope;
mod background;
mod border;
mod clip;
mod events;
mod frame;
mod geometry;
mod padding;
mod presentation;
mod repaint_boundary;
mod safe_area;
mod size;

pub use alignment::{AlignmentFrame, AlignmentRenderObject};
pub use animation_scope::AnimationScope;
pub use background::{Background, BackgroundRenderObject};
pub use border::{Border, BorderRenderObject};
pub use clip::{Clip, ClipRenderObject};
pub use events::{
    Focusable, FocusableRenderObject, OnClick, OnClickRenderObject, OnEvent, OnEventRenderObject,
    OnExit, OnExitRenderObject, OnHover, OnHoverRenderObject, OnKey, OnKeyRenderObject,
    OnMouseButton, OnMouseButtonRenderObject, OnMouseDelta, OnMouseDeltaRenderObject, OnMouseMove,
    OnMouseMoveRenderObject,
};
pub use frame::{Frame, FrameRenderObject};
pub use geometry::{OnGeometryChange, OnGeometryChangeRenderObject};
pub use padding::{Padding, PaddingRenderObject};
pub use presentation::{OffsetRenderObject, OffsetView, Opacity, OpacityRenderObject};
pub use repaint_boundary::{RepaintBoundary, RepaintBoundaryRenderObject};
pub use safe_area::{SafeArea, SafeAreaRenderObject};
pub use size::{SetSize, SizeRenderObject};

mod context_menu;
pub use context_menu::ContextMenu;
