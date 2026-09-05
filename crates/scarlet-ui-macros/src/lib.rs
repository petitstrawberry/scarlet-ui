//! ScarletUI Macros - Procedural macros for ScarletUI
//!
//! This crate provides derive macros for ScarletUI traits.

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::parse::Parser;
use syn::punctuated::Punctuated;
use syn::{
    Data, DataStruct, DeriveInput, Expr, Fields, ItemFn, Lit, LitStr, Meta, Token, TypePath,
    parse_macro_input,
};

/// Derive macro for View trait
///
/// # Example
///
/// ```rust
/// use scarlet_ui::{State, Text, View as _};
/// use scarlet_ui_macros::View;
///
/// #[derive(View, Clone)]
/// #[view(body = content)]
/// struct CounterApp {
///     count: State<i32>,
/// }
///
/// impl CounterApp {
///     fn content(&self) -> Text {
///         Text::new("Counter")
///     }
/// }
///
/// let app = CounterApp::default();
/// assert_eq!(app.count.get(), 0);
/// assert_eq!(app.listenables().len(), 1);
/// let element = app.create_element();
/// ```
///
/// This macro generates:
/// - `impl View for CounterApp` - collects listenables and creates ComponentElement when a body is specified
/// - `impl Default for CounterApp` - auto-initializes State fields with auto-generated StateId
///
/// The type must implement `Clone`. Reusable components select an inherent
/// method with `#[view(body = method_name)]`. It takes `&self` and returns an
/// owned `View + Clone + 'static`. The child is built from that method, not
/// from the derived view itself.
///
/// Application state containers can omit the body attribute: the application
/// runner subscribes to their listenables and builds windows from `scenes()`.
/// A body-less derive cannot itself be mounted as a child View.
///
/// # Panics
///
/// Calling the generated `create_element()` without a body attribute panics
/// with a configuration diagnostic. Application startup does not call it.
///
/// Users can implement their own `new()` method and use `Default::default()`:
/// ```rust
/// use scarlet_ui::{Application, Scene, State, Text, Window, WindowGroup};
/// use scarlet_ui_macros::View;
///
/// #[derive(View, Clone)]
/// struct MyApp {
///     count: State<i32>,
/// }
///
/// impl MyApp {
///     fn new() -> Self {
///         Self::default()
///     }
/// }
///
/// impl Application for MyApp {
///     fn scenes(&self) -> impl Scene {
///         WindowGroup::new("main", Window::new("My app", Text::new("Hello")))
///     }
/// }
///
/// assert_eq!(MyApp::new().count.get(), 0);
/// ```
///
/// # Note for `#![no_std]` environments
///
/// The generated implementation does not require the caller's std prelude.
/// Select the UI/runtime features separately: normal Scarlet targets support
/// `std`, while legacy `no_std` binaries still provide their Scarlet runtime
/// through `scarlet_std`. This host doctest checks the caller's `no_std` syntax,
/// not a freestanding binary's runtime startup.
///
/// ```rust
/// #![no_std]
/// # extern crate std; // Host rustdoc executable; not part of the library example.
/// use scarlet_ui::{State, StateId, Text};
/// use scarlet_ui_macros::View;
///
/// #[derive(View, Clone)]
/// #[view(body = content)]
/// struct CounterApp {
///     count: State<i32>,
/// }
///
/// impl CounterApp {
///     pub fn new(custom_value: i32) -> Self {
///         Self {
///             count: State::new(StateId::new(0), custom_value),
///         }
///     }
///
///     fn content(&self) -> Text {
///         Text::new("Counter")
///     }
/// }
/// # fn main() {
/// #     let app = CounterApp::new(42);
/// #     assert_eq!(app.count.get(), 42);
/// #     let _element = scarlet_ui::View::create_element(&app);
/// # }
/// ```
#[proc_macro_derive(View, attributes(view))]
pub fn derive_view(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let body = match view_body(&input) {
        Ok(body) => body,
        Err(error) => return error.into_compile_error().into(),
    };
    let create_element = match body {
        Some(method) => quote! {
            ::scarlet_ui::__private::Box::new(
                ::scarlet_ui::element::ComponentElement::new_with_builder(
                    self.clone(),
                    |view| ::scarlet_ui::__private::Box::new(view.#method()),
                ),
            )
        },
        None => quote! {
            ::core::panic!(
                "a body-less #[derive(View)] is application state, not a mountable component; \
                 add #[view(body = method_name)] to specify its child View"
            )
        },
    };

    // Parse struct fields to find State<T> fields
    let (state_fields, state_indices) = match &input.data {
        Data::Struct(DataStruct { fields, .. }) => extract_state_fields_with_indices(fields),
        _ => {
            // For enums or other types, return empty
            (Punctuated::new(), Vec::new())
        }
    };

    // Generate code to collect State fields
    let collect_state_fields: std::vec::Vec<proc_macro2::TokenStream> = state_fields
        .iter()
        .map(|field_name| {
            let field_ident = quote::format_ident!("{}", field_name);
            quote! {
                vec.push(&self.#field_ident as &dyn ::scarlet_ui::state::Listenable);
            }
        })
        .collect();

    // Generate Default implementation that initializes State fields with auto-generated StateId
    let default_init: std::vec::Vec<proc_macro2::TokenStream> = state_indices
        .iter()
        .zip(state_fields.iter())
        .map(|(idx, field_name)| {
            let field_ident = quote::format_ident!("{}", field_name);
            let idx_as_u32 = *idx as u32;
            // Use State::initial for types with Default (State<T> inner type)
            quote! {
                #field_ident: ::scarlet_ui::state::State::initial(
                    ::scarlet_ui::state::StateId::new(#idx_as_u32)
                ),
            }
        })
        .collect();

    let has_state_fields = !state_fields.is_empty();

    let default_impl = if has_state_fields {
        quote! {
            impl core::default::Default for #name {
                fn default() -> Self {
                    Self {
                        #(#default_init)*
                    }
                }
            }
        }
    } else {
        quote! {}
    };

    let expanded = quote! {
        #default_impl

        impl ::scarlet_ui::view::View for #name {
            fn create_element(&self) -> ::scarlet_ui::__private::Box<dyn ::scarlet_ui::element::Element> {
                #create_element
            }

            fn listenables(&self) -> ::scarlet_ui::__private::Vec<&dyn ::scarlet_ui::state::Listenable> {
                let mut vec = ::scarlet_ui::__private::Vec::new();
                #(#collect_state_fields)*
                vec
            }

            fn as_any(&self) -> &dyn core::any::Any {
                self
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}

fn view_body(input: &DeriveInput) -> syn::Result<Option<syn::Ident>> {
    let mut body = None;
    for attr in input
        .attrs
        .iter()
        .filter(|attr| attr.path().is_ident("view"))
    {
        attr.parse_nested_meta(|meta| {
            if !meta.path.is_ident("body") {
                return Err(meta.error("expected `body = method_name`"));
            }
            if body.is_some() {
                return Err(meta.error("duplicate view body"));
            }
            body = Some(meta.value()?.parse()?);
            Ok(())
        })?;
    }
    Ok(body)
}

/// Register a function as a ScarletUI preview.
///
/// The function must return a `View + Clone + 'static` value. Multiple preview
/// functions can be registered in the same crate; the preview wrapper exports a
/// single dylib entrypoint that exposes all registered previews.
/// The consuming crate must enable its `preview` feature; without it the
/// annotated functions and registrations are omitted. The macro crate's
/// matching feature lets CI exercise these examples with registration enabled.
///
/// # Example
///
/// ```rust
/// use scarlet_ui::{Text, View};
/// # #[derive(Default)]
/// # struct CounterApp;
/// # impl CounterApp {
/// #     fn content(&self) -> Text { Text::new("Counter") }
/// #     fn compact_content(&self) -> Text { Text::new("Compact counter") }
/// # }
///
/// #[scarlet_ui::preview]
/// fn counter_preview() -> impl View + Clone {
///     CounterApp::default().content()
/// }
///
/// #[scarlet_ui::preview]
/// fn compact_counter_preview() -> impl View + Clone {
///     CounterApp::default().compact_content()
/// }
///
/// #[scarlet_ui::preview(width = 320.0, height = 180.0)]
/// fn button_preview() -> impl View + Clone {
///     scarlet_ui::Button::new("OK")
/// }
///
/// # let mut library = scarlet_ui::preview::registered_preview_library();
/// # let previews = library.previews();
/// # #[cfg(feature = "preview")]
/// # {
/// #     assert_eq!(previews.len(), 3);
/// #     let button = previews.iter().find(|entry| entry.name == "Button Preview")
/// #         .expect("the attribute must register the button factory");
/// #     let session = library.create(&button.id, Default::default())
/// #         .expect("a registered factory must create a session");
/// #     assert_eq!(session.title(), "Button Preview");
/// #     assert_eq!(session.size(), scarlet_ui::Size::new(320.0, 180.0));
/// # }
/// # #[cfg(not(feature = "preview"))]
/// # assert!(previews.is_empty(), "preview-disabled crates must not register factories");
/// ```
#[proc_macro_attribute]
pub fn preview(attr: TokenStream, input: TokenStream) -> TokenStream {
    let item = parse_macro_input!(input as ItemFn);
    let name = &item.sig.ident;
    let args = preview_args(attr, &name.to_string());
    let display_name = LitStr::new(&args.name, name.span());
    let width = args.width;
    let height = args.height;
    let create_name = format_ident!("__scarlet_ui_preview_create_{}", name);

    let expanded = quote! {
        #[cfg(feature = "preview")]
        #item

        #[doc(hidden)]
        #[cfg(feature = "preview")]
        fn #create_name(
            context: ::scarlet_ui::preview::PreviewCreateContext,
        ) -> ::scarlet_ui::__private::Box<dyn ::scarlet_ui::preview::PreviewSession> {
            ::scarlet_ui::preview::preview_session_from_view(
                #display_name,
                #name(),
                context,
            )
        }

        #[cfg(feature = "preview")]
        ::scarlet_ui::__private::inventory::submit! {
            ::scarlet_ui::preview::PreviewRegistration {
                id: concat!(module_path!(), "::", stringify!(#name)),
                name: #display_name,
                preferred_size: ::scarlet_ui::geometry::Size::new(#width as f32, #height as f32),
                create: #create_name,
            }
        }
    };

    proc_macro::TokenStream::from(expanded)
}

struct PreviewArgs {
    name: String,
    width: Expr,
    height: Expr,
}

fn preview_args(attr: TokenStream, fallback: &str) -> PreviewArgs {
    let args = PreviewArgs {
        name: humanize_preview_name(fallback),
        width: syn::parse_quote!(0.0),
        height: syn::parse_quote!(0.0),
    };
    if attr.is_empty() {
        return args;
    }
    let parser = Punctuated::<Meta, Token![,]>::parse_terminated;
    let Ok(args) = parser.parse(attr) else {
        return args;
    };
    let mut preview_args = PreviewArgs {
        name: humanize_preview_name(fallback),
        width: syn::parse_quote!(0.0),
        height: syn::parse_quote!(0.0),
    };
    for arg in args {
        let Meta::NameValue(name_value) = arg else {
            continue;
        };
        if name_value.path.is_ident("name") {
            if let Expr::Lit(expr_lit) = name_value.value
                && let Lit::Str(value) = expr_lit.lit
            {
                preview_args.name = value.value();
            }
        } else if name_value.path.is_ident("width") {
            preview_args.width = name_value.value;
        } else if name_value.path.is_ident("height") {
            preview_args.height = name_value.value;
        }
    }
    preview_args
}

fn humanize_preview_name(name: &str) -> String {
    let mut output = String::new();
    for word in name.split('_').filter(|word| !word.is_empty()) {
        if !output.is_empty() {
            output.push(' ');
        }
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            output.extend(first.to_uppercase());
            output.push_str(chars.as_str());
        }
    }
    if output.is_empty() {
        name.to_string()
    } else {
        output
    }
}

/// Extract field names that are of type State<T> (without types)
fn extract_state_fields(fields: &Fields) -> Punctuated<syn::Ident, syn::token::Comma> {
    let mut state_fields = Punctuated::new();

    if let Fields::Named(named_fields) = fields {
        for field in &named_fields.named {
            let field_name = field.ident.as_ref().unwrap();

            // Check if field type is State<T>
            if is_state_type(&field.ty) {
                state_fields.push(field_name.clone());
            }
        }
    }

    state_fields
}

/// Extract field names and indices for State<T> fields (with auto-incrementing IDs)
fn extract_state_fields_with_indices(
    fields: &Fields,
) -> (Punctuated<syn::Ident, syn::token::Comma>, Vec<usize>) {
    let state_fields = extract_state_fields(fields);
    let state_indices = (0..state_fields.len()).collect();
    (state_fields, state_indices)
}

/// Check if a type is State<T> (either scarlet_ui::state::State or just State)
fn is_state_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(TypePath { path, .. }) = ty {
        // Get the last segment of the path
        if let Some(last_segment) = path.segments.last() {
            // Check if it's "State"
            if last_segment.ident == "State" {
                // Optionally check if it's from scarlet_ui::state module
                // For simplicity, we accept any "State" identifier
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::view_body;

    #[test]
    fn view_body_is_optional_for_application_state() {
        let input = syn::parse_quote!(
            struct App;
        );
        assert!(
            view_body(&input)
                .expect("valid application derive")
                .is_none()
        );
    }

    #[test]
    fn view_body_selects_the_named_method() {
        let input = syn::parse_quote!(
            #[view(body = build_counter)]
            struct Counter;
        );
        let body = view_body(&input)
            .expect("valid body selector")
            .expect("body method");
        assert_eq!(body, "build_counter");
    }

    #[test]
    fn view_body_rejects_unknown_or_malformed_options() {
        for attr in [
            "#[view(content = body)]",
            "#[view(body)]",
            "#[view(body = \"content\")]",
            "#[view(body = 42)]",
        ] {
            let input = syn::parse_str(&format!("{attr} struct Counter;"))
                .expect("syntactically valid derive input");
            assert!(
                view_body(&input).is_err(),
                "accepted malformed option {attr}"
            );
        }
    }

    #[test]
    fn view_body_rejects_duplicate_selectors() {
        for attrs in [
            "#[view(body = first, body = second)]",
            "#[view(body = first)] #[view(body = second)]",
        ] {
            let input = syn::parse_str(&format!("{attrs} struct Counter;"))
                .expect("syntactically valid derive input");
            let error = view_body(&input).expect_err("duplicate body must be rejected");
            assert!(error.to_string().contains("duplicate view body"));
        }
    }
}
