//! Per-window adaptive layout traits.
//!
//! Device posture and input capabilities describe the surrounding hardware,
//! but they do not describe how much room a particular window has. These
//! traits are therefore derived from the logical bounds supplied to each
//! window or container during layout.

use crate::geometry::Size;

/// Horizontal space available to one window or container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HorizontalSizeClass {
    /// Phone-like or narrow split-window width.
    Compact,
    /// Ordinary tablet or medium desktop-window width.
    Regular,
    /// Wide window with room for multi-column content.
    Expanded,
}

/// Vertical space available to one window or container.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerticalSizeClass {
    /// A short landscape or heavily constrained window.
    Compact,
    /// A window with ordinary vertical working space.
    Regular,
}

/// Stable adaptive traits derived from one logical layout size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WindowSizeClass {
    /// Horizontal size class.
    pub horizontal: HorizontalSizeClass,
    /// Vertical size class.
    pub vertical: VerticalSizeClass,
}

impl WindowSizeClass {
    /// Minimum width for regular two-region layouts.
    pub const REGULAR_MIN_WIDTH: f32 = 600.0;
    /// Minimum width for expanded multi-column layouts.
    pub const EXPANDED_MIN_WIDTH: f32 = 900.0;
    /// Minimum height for regular vertical layouts.
    pub const REGULAR_MIN_HEIGHT: f32 = 480.0;

    /// Resolve adaptive traits for a logical size.
    ///
    /// # Arguments
    ///
    /// * `size` - Logical bounds available to one window or container.
    ///
    /// # Returns
    ///
    /// Size classes derived only from the supplied bounds.
    pub const fn for_size(size: Size) -> Self {
        let horizontal = if size.width >= Self::EXPANDED_MIN_WIDTH {
            HorizontalSizeClass::Expanded
        } else if size.width >= Self::REGULAR_MIN_WIDTH {
            HorizontalSizeClass::Regular
        } else {
            HorizontalSizeClass::Compact
        };
        let vertical = if size.height >= Self::REGULAR_MIN_HEIGHT {
            VerticalSizeClass::Regular
        } else {
            VerticalSizeClass::Compact
        };
        Self {
            horizontal,
            vertical,
        }
    }

    /// Return whether the bounds are taller than they are wide.
    ///
    /// # Arguments
    ///
    /// * `size` - Logical bounds used to determine orientation.
    ///
    /// # Returns
    ///
    /// `true` for portrait-like bounds.
    pub const fn is_portrait(size: Size) -> bool {
        size.height > size.width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn size_classes_follow_each_window_bounds() {
        assert_eq!(
            WindowSizeClass::for_size(Size::new(390.0, 844.0)),
            WindowSizeClass {
                horizontal: HorizontalSizeClass::Compact,
                vertical: VerticalSizeClass::Regular,
            }
        );
        assert_eq!(
            WindowSizeClass::for_size(Size::new(768.0, 1024.0)),
            WindowSizeClass {
                horizontal: HorizontalSizeClass::Regular,
                vertical: VerticalSizeClass::Regular,
            }
        );
        assert_eq!(
            WindowSizeClass::for_size(Size::new(1024.0, 360.0)),
            WindowSizeClass {
                horizontal: HorizontalSizeClass::Expanded,
                vertical: VerticalSizeClass::Compact,
            }
        );
    }
}
