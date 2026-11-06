//! Compare real controls with the shared font measurement and glyph layout.
//! `cargo run -p scarlet-ui --example typography-smoke --features platform-winit`
//! Set SCARLET_TYPOGRAPHY_FONT_DIR to Scarlet's bundled share/fonts directory
//! for reproducible primary/fallback selection. No documents are written.
use scarlet_ui::{graphics, prelude::*, vstack};
use scarlet_ui_macros::View;

const LATIN: &str = "AVATAR WiFi minimum 0123456789";
const JAPANESE: &str = "日本語の文字幅、句読点。「確認」";
const MIXED: &str = "Scarlet 日本語 123 — [OK]（保存）";
const FALLBACK: &str = "Fallback: ☃ ⏻ \u{f120}";

#[derive(Clone, View)]
struct TypographyProbe {
    field: State<String>,
    document: State<TextDocument>,
    selection: State<TextSelection>,
}

impl Application for TypographyProbe {
    fn scenes(&self) -> impl Scene {
        Window::new(
            "ScarletUI typography verification",
            vstack! {
                Text::new("Typography: bundled fonts, unchanged spacing").font_size(20.),
                Text::new("Labels at 12 / 14 / 18 / 24 logical pixels").font_size(12.),
                vstack! {
                    Text::new(LATIN).font_size(12.),
                    Text::new(LATIN).font_size(14.),
                    Text::new(JAPANESE).font_size(18.),
                    Text::new(MIXED).font_size(24.),
                    Text::new(FALLBACK).font_size(18.),
                }.spacing(10.),
                Button::new(MIXED).font_size(14.),
                TextField::new(self.field.clone()).font_size(14.),
                TextView::with_document(self.document.clone(), self.selection.clone())
                    .font_size(14.)
                    .frame_height(140.),
                Text::new("Tab moves focus; input and multiline text remain editable.").font_size(12.),
            }.spacing(10.).padding(20.),
        ).size(Size::new(700., 620.))
    }
}

fn main() {
    if let Some(directory) = std::env::var_os("SCARLET_TYPOGRAPHY_FONT_DIR") {
        let directory = std::path::Path::new(&directory);
        let font = |name| -> &'static [u8] {
            Box::leak(
                std::fs::read(directory.join(name))
                    .expect("font fixture")
                    .into_boxed_slice(),
            )
        };
        let mut stack = graphics::FontStack::new(font("Mplus1-Regular.ttf")).unwrap();
        for name in [
            "NotoSansCJKjp-Regular.otf",
            "NotoSansSymbols2-Regular.ttf",
            "JetBrainsMonoNerdFontMono-Regular.ttf",
        ] {
            stack.add_fallback(font(name)).unwrap();
        }
        graphics::set_default_font_stack(stack);
    }
    for sample in [LATIN, JAPANESE, MIXED, FALLBACK] {
        for size in [12., 14., 18., 24.] {
            let measured = graphics::measure_text_sized(sample, size);
            for dpi in [1000, 1250, 2000] {
                let glyphs = graphics::rasterize_text(sample, size, dpi);
                let ink_right = glyphs
                    .iter()
                    .map(|g| g.x + g.width as i32)
                    .max()
                    .unwrap_or(0);
                println!(
                    "text={sample:?} em={size} dpi={dpi} measured={measured:?} ink_right={ink_right} glyph_positions={:?}",
                    glyphs
                        .iter()
                        .map(|g| (g.key.font_slot, g.x, g.width))
                        .collect::<Vec<_>>()
                );
            }
        }
    }
    if std::env::args().any(|arg| arg == "--metrics-only") {
        return;
    }
    TypographyProbe {
        field: State::new(StateId::new(100), MIXED.to_owned()),
        document: State::new(
            StateId::new(101),
            TextDocument::from_str(&format!("{LATIN}\n{JAPANESE}\n{MIXED}\n{FALLBACK}")),
        ),
        selection: State::new(StateId::new(102), TextSelection::collapsed(0)),
    }
    .run()
    .unwrap();
}
