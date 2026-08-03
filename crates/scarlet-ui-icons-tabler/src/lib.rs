//! Selected Tabler Icons vector data for Scarlet UI.
//!
//! This crate contains no renderer. Scarlet UI consumes the generated path
//! commands and owns rasterization, caching, theme color, and DPI handling.

#![no_std]

/// One SVG path command in Tabler's 24×24 coordinate system.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum IconCommand {
    /// Begin a new subpath.
    MoveTo(f32, f32),
    /// Add a straight segment.
    LineTo(f32, f32),
    /// Add a quadratic Bézier segment.
    QuadTo {
        /// Control point X coordinate.
        control_x: f32,
        /// Control point Y coordinate.
        control_y: f32,
        /// End point X coordinate.
        x: f32,
        /// End point Y coordinate.
        y: f32,
    },
    /// Add a cubic Bézier segment.
    CubicTo {
        /// First control point X coordinate.
        control_1_x: f32,
        /// First control point Y coordinate.
        control_1_y: f32,
        /// Second control point X coordinate.
        control_2_x: f32,
        /// Second control point Y coordinate.
        control_2_y: f32,
        /// End point X coordinate.
        x: f32,
        /// End point Y coordinate.
        y: f32,
    },
    /// Add an elliptical arc segment.
    ArcTo {
        /// Horizontal radius.
        radius_x: f32,
        /// Vertical radius.
        radius_y: f32,
        /// X-axis rotation in degrees.
        rotation: f32,
        /// Whether the large arc is selected.
        large_arc: bool,
        /// Whether the positive-angle sweep is selected.
        sweep: bool,
        /// End point X coordinate.
        x: f32,
        /// End point Y coordinate.
        y: f32,
    },
    /// Close the current subpath.
    Close,
    /// Finish one SVG path element.
    ///
    /// This preserves fill-rule boundaries when an icon contains multiple
    /// overlapping path elements. Outline renderers may ignore it.
    EndPath,
}

include!(concat!(env!("OUT_DIR"), "/generated.rs"));
