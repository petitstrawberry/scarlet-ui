//! Either - Conditional view for branching UI
//!
//! Similar to SwiftUI's `_ConditionalContent`, this allows `if`/`match`
//! expressions to return different view types from a single branch.
//!
//! # Example
//!
//! ```rust
//! use scarlet_ui_core::{Rectangle, Text};
//! use scarlet_ui_core::views::Either;
//! # let show = true;
//! # fn settings_view() -> Text { Text::new("Settings") }
//! # fn clock_view() -> Rectangle { Rectangle::new() }
//!
//! match show {
//!     true => Either::A(settings_view()),
//!     false => Either::B(clock_view()),
//! };
//! ```
//!
//! For 3+ branches, use `Either3`, `Either4`, etc.:
//!
//! ```rust
//! use scarlet_ui_core::{Rectangle, Text};
//! use scarlet_ui_core::views::Either3;
//! # let page = 0;
//! # fn home() -> Text { Text::new("Home") }
//! # fn settings() -> Rectangle { Rectangle::new() }
//! # fn about() -> Text { Text::new("About") }
//!
//! match page {
//!     0 => Either3::A(home()),
//!     1 => Either3::B(settings()),
//!     _ => Either3::C(about()),
//! };
//! ```

use alloc::boxed::Box;
use alloc::vec::Vec;
use core::any::Any;

use crate::element::{ComponentElement, Element};
use crate::view::View;

macro_rules! define_either {
    ($name:ident, ($($t:ident),+ $(,)?)) => {
        #[derive(Clone)]
        pub enum $name<$($t),+> {
            $($t($t)),+
        }

        impl<$($t: View + Clone + 'static),+> View for $name<$($t),+> {
            fn create_element(&self) -> Box<dyn Element> {
                Box::new(ComponentElement::new_with_builder(
                    self.clone(),
                    |view| match view {
                        $(Self::$t(value) => value.clone_view(),)+
                    },
                ))
            }

            fn listenables(&self) -> Vec<&dyn crate::state::Listenable> {
                // The mounted branch owns its own subscriptions. The parent
                // that chooses the branch is responsible for rebuilding this
                // conditional value when the choice changes.
                Vec::new()
            }

            fn as_any(&self) -> &dyn Any {
                self
            }
        }
    };
}

define_either!(Either, (A, B));
define_either!(Either3, (A, B, C));
define_either!(Either4, (A, B, C, D));
define_either!(Either5, (A, B, C, D, E));
define_either!(Either6, (A, B, C, D, E, F));
