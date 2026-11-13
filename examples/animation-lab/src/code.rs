//! In-app, read-only source walkthroughs shared by the native and Web demos.
use crate::{
    lab::Views,
    source::{ExampleId, SourceFile},
};
use scarlet_ui::{
    clipboard::{ClipboardHandle, TextClipboard},
    element::{Element, ElementRenderObject, LayoutConstraints, RenderElement, UpdateResult},
    graphics::{self, FontStack},
    prelude::*,
    renderer::PaintContext,
    state::{Listenable, generate_state_id},
};
use std::{
    any::Any,
    cell::RefCell,
    rc::Rc,
    sync::{Arc, OnceLock},
};

pub(crate) const SURFACE: Color = Color::rgb_f32(0.965, 0.973, 0.988);
const INK: Color = Color::rgb_f32(0.12, 0.17, 0.26);
const MUTED: Color = Color::rgb_f32(0.32, 0.38, 0.48);
const CODE: Color = Color::rgb_f32(0.065, 0.09, 0.15);
const FONT_SIZE: f32 = 13.0;
const LINE_HEIGHT: f32 = 21.0;
const GUTTER: f32 = 54.0;

#[derive(Clone)]
pub(crate) struct CodeBrowser {
    pub(crate) selected: State<Option<ExampleId>>,
    file: State<Option<SourceFile>>,
    wrap: State<bool>,
    feedback: State<String>,
    clipboard: Rc<RefCell<Option<TextClipboard>>>,
    pending: Rc<RefCell<Option<ClipboardHandle<()>>>>,
}

impl Default for CodeBrowser {
    fn default() -> Self {
        Self {
            selected: State::new(generate_state_id(), None),
            file: State::new(generate_state_id(), None),
            wrap: State::new(generate_state_id(), true),
            feedback: State::new(generate_state_id(), String::new()),
            clipboard: Rc::new(RefCell::new(None)),
            pending: Rc::new(RefCell::new(None)),
        }
    }
}

impl CodeBrowser {
    pub(crate) fn is_open(&self) -> bool {
        self.selected.get().is_some()
    }
    pub(crate) fn open(&self, example: ExampleId) {
        self.file.set(None);
        self.feedback.set(String::new());
        self.selected.set(Some(example));
    }
    pub(crate) fn close(&self) {
        self.selected.set(None);
    }
    pub(crate) fn location_key(&self) -> String {
        format!("source-{:?}-{:?}", self.selected.get(), self.file.get())
    }
    pub(crate) fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.selected, &self.file, &self.wrap, &self.feedback]
    }
    pub(crate) fn attach_clipboard(&self, clipboard: Option<TextClipboard>) {
        *self.clipboard.borrow_mut() = clipboard;
    }
    pub(crate) fn poll_copy(&self) {
        let result = self
            .pending
            .borrow()
            .as_ref()
            .and_then(|handle| handle.try_take());
        if let Some(result) = result {
            self.pending.borrow_mut().take();
            self.feedback.set(match result {
                Ok(()) => "Copied.".into(),
                Err(error) => format!("Could not copy: {error}"),
            });
        }
    }
    fn copy(&self, text: String) {
        self.feedback.set("Copying...".into());
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(clipboard) = self.clipboard.borrow().as_ref() {
            *self.pending.borrow_mut() = Some(clipboard.write_text(text));
        } else {
            self.feedback
                .set("Clipboard unavailable on this platform.".into());
        }
        #[cfg(target_arch = "wasm32")]
        {
            let feedback = self.feedback.clone();
            let Some(window) = web_sys::window() else {
                return;
            };
            let clipboard = window.navigator().clipboard();
            let value: &wasm_bindgen::JsValue = clipboard.as_ref();
            if value.is_undefined() || value.is_null() {
                feedback.set("Clipboard unavailable in this browser.".into());
                return;
            }
            // Request the write synchronously within the trusted button event.
            let promise = clipboard.write_text(&text);
            wasm_bindgen_futures::spawn_local(async move {
                let result = wasm_bindgen_futures::JsFuture::from(promise).await;
                feedback.set(
                    if result.is_ok() {
                        "Copied."
                    } else {
                        "Copy blocked by the browser. Allow clipboard access and retry."
                    }
                    .into(),
                );
            });
        }
    }
    pub(crate) fn button(
        &self,
        id: ExampleId,
        label: &str,
        width: f32,
    ) -> impl View + Clone + 'static + use<> {
        let browser = self.clone();
        action_button(label)
            .on_click(move || browser.open(id))
            .frame(width, 38.0)
    }
    pub(crate) fn back_button(&self) -> impl View + Clone + 'static + use<> {
        let browser = self.clone();
        Button::new("Back to demo")
            .header_style()
            .icon(Icon::ChevronLeft)
            .on_click(move || browser.close())
    }
    pub(crate) fn page(&self, width: f32) -> impl View + Clone + 'static + use<> {
        CodePage {
            browser: self.clone(),
            width,
        }
    }
}

fn action_button(label: &str) -> Button {
    Button::new(label)
        .font_size(13.0)
        .background_color(Color::rgb(227u8, 234u8, 248u8))
        .border_color(Color::CLEAR)
        .text_color(INK)
}

#[derive(Clone)]
struct CodePage {
    browser: CodeBrowser,
    width: f32,
}
impl View for CodePage {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(scarlet_ui::ComponentElement::new_with_builder(
            self.clone(),
            |page| Box::new(page.content()),
        ))
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        self.browser.listenables()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
impl CodePage {
    fn content(&self) -> impl View + Clone + 'static {
        let padding = if self.width < 600.0 { 16.0 } else { 24.0 };
        let width = (self.width - padding * 2.0).max(0.0);
        let mut rows: Vec<Box<dyn View>> = Vec::new();
        if let Some(id) = self.browser.selected.get() {
            let example = id.example();
            rows.push(Box::new(prose(example.title, width, 25.0, INK)));
            rows.push(Box::new(prose(example.summary, width, 14.0, MUTED)));
            let mut files = Vec::new();
            for section in &example.sections {
                if !files.contains(&section.file) {
                    files.push(section.file);
                }
            }
            let mut modes: Vec<(String, Option<SourceFile>)> = vec![("Walkthrough".into(), None)];
            modes.extend(
                files
                    .into_iter()
                    .map(|file| (file.name().into(), Some(file))),
            );
            let columns = if width >= 620.0 {
                modes.len()
            } else if width >= 300.0 {
                2
            } else {
                1
            };
            let cell = ((width - 8.0 * (columns - 1) as f32) / columns as f32).max(0.0);
            let tabs: Vec<Box<dyn View>> = modes
                .into_iter()
                .map(|(label, file)| {
                    let browser = self.browser.clone();
                    let selected = browser.file.get() == file;
                    Box::new(
                        action_button(&label)
                            .background_color(if selected { INK } else { Color::WHITE })
                            .text_color(if selected { Color::WHITE } else { INK })
                            .on_click(move || {
                                browser.file.set(file);
                                browser.feedback.set(String::new());
                            })
                            .frame(cell, 38.0),
                    ) as Box<dyn View>
                })
                .collect();
            for chunk in tabs.chunks(columns) {
                rows.push(Box::new(HStack::new(Views(chunk.to_vec())).spacing(8.0)));
            }
            let browser = self.browser.clone();
            rows.push(Box::new(
                action_button(if browser.wrap.get() {
                    "Wrap: on"
                } else {
                    "Wrap: off"
                })
                .on_click(move || browser.wrap.update(|wrap| *wrap = !*wrap))
                .frame(124.0, 36.0),
            ));
            if !self.browser.feedback.get().is_empty() {
                rows.push(Box::new(prose(
                    &self.browser.feedback.get(),
                    width,
                    13.0,
                    MUTED,
                )));
            }
            if let Some(file) = self.browser.file.get() {
                rows.push(Box::new(prose("Full source from this build. Copy this file together with its referenced modules.", width, 13.0, MUTED)));
                self.append_code(
                    &mut rows,
                    file.name(),
                    file.contents(),
                    width,
                    560.0,
                    "Copy file",
                );
            } else {
                for (index, step) in example.steps.iter().enumerate() {
                    rows.push(Box::new(prose(
                        &format!("{}. {}", index + 1, step),
                        width,
                        14.0,
                        INK,
                    )));
                }
                rows.push(Box::new(prose("Excerpts from the running demo; shared types and helpers are in the full source tabs.", width, 12.0, MUTED)));
                for (index, section) in example.sections.iter().enumerate() {
                    rows.push(Box::new(prose(
                        &format!("{:02}  {}", index + 1, section.title),
                        width,
                        17.0,
                        INK,
                    )));
                    self.append_code(
                        &mut rows,
                        section.file.name(),
                        &section.code,
                        width,
                        420.0,
                        "Copy excerpt",
                    );
                }
            }
        }
        VStack::new(Views(rows))
            .alignment(Alignment::Leading)
            .spacing(14.0)
            .padding(padding)
            .frame_width(self.width)
    }
    fn append_code(
        &self,
        rows: &mut Vec<Box<dyn View>>,
        name: &str,
        code: &str,
        width: f32,
        max_height: f32,
        copy_label: &str,
    ) {
        let browser = self.browser.clone();
        let copied = code.to_owned();
        rows.push(Box::new(scarlet_ui::hstack! {
            Text::new(name).font_size(12.0).color(MUTED).frame_width((width - 134.0).max(0.0)),
            action_button(copy_label).on_click(move || browser.copy(copied.clone())).frame(126.0, 34.0),
        }.spacing(8.0)));
        let text = CodeText::new(code, width, self.browser.wrap.get());
        let size = text.size;
        rows.push(Box::new(
            ScrollView::new(text)
                .both_axes()
                .content_size(size.width, size.height)
                .content_safe_area(scarlet_ui::SafeAreaEdges::NONE)
                .frame(width, size.height.min(max_height).max(64.0))
                .background(CODE)
                .clip_radius(12.0),
        ));
    }
}

fn prose(text: &str, width: f32, size: f32, color: Color) -> impl View + Clone + 'static + use<> {
    let mut output = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let candidate = if line.is_empty() {
            word.to_owned()
        } else {
            format!("{line} {word}")
        };
        if !line.is_empty() && graphics::measure_text_sized(&candidate, size).0 as f32 > width {
            output.push_str(&line);
            output.push('\n');
            line = word.to_owned();
        } else {
            line = candidate;
        }
    }
    output.push_str(&line);
    Text::new(output)
        .font_size(size)
        .color(color)
        .frame_width(width)
}

fn font() -> Arc<FontStack> {
    static FONT: OnceLock<Arc<FontStack>> = OnceLock::new();
    FONT.get_or_init(|| {
        Arc::new(
            FontStack::new(include_bytes!("../assets/DejaVuSansMono.ttf"))
                .expect("bundled code font"),
        )
    })
    .clone()
}

#[derive(Clone)]
struct CodeLine {
    number: Option<usize>,
    text: String,
}
#[derive(Clone)]
struct CodeText {
    lines: Arc<Vec<CodeLine>>,
    size: Size,
}
impl CodeText {
    fn new(code: &str, width: f32, wrap: bool) -> Self {
        let font = font();
        let cell = graphics::measure_text_sized_with_font_stack("MMMMMMMMMM", FONT_SIZE, &font).0
            as f32
            / 10.0;
        let columns = (((width - GUTTER - 16.0).max(cell)) / cell)
            .floor()
            .max(1.0) as usize;
        let mut lines = Vec::new();
        let mut widest = 0.0f32;
        for (index, line) in code.lines().enumerate() {
            let line = line.replace('\t', "    ");
            let chars: Vec<char> = line.chars().collect();
            let chunks: Vec<String> = if wrap && !chars.is_empty() {
                chars
                    .chunks(columns)
                    .map(|chunk| chunk.iter().collect())
                    .collect()
            } else {
                vec![line]
            };
            for (part, text) in chunks.into_iter().enumerate() {
                widest = widest.max(
                    graphics::measure_text_sized_with_font_stack(&text, FONT_SIZE, &font).0 as f32,
                );
                lines.push(CodeLine {
                    number: (part == 0).then_some(index + 1),
                    text,
                });
            }
        }
        let size = Size::new(
            width.max(GUTTER + widest + 16.0),
            24.0 + lines.len().max(1) as f32 * LINE_HEIGHT,
        );
        Self {
            lines: Arc::new(lines),
            size,
        }
    }
}
impl View for CodeText {
    fn create_element(&self) -> Box<dyn Element> {
        Box::new(RenderElement::with_view_children_and_updater(
            self.clone(),
            |view| CodePaint(view.clone()),
            |render, view| {
                render.0 = view.clone();
                UpdateResult::Updated
            },
            |_| Vec::new(),
        ))
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}
struct CodePaint(CodeText);
impl ElementRenderObject for CodePaint {
    fn layout(&mut self, constraints: LayoutConstraints) -> Size {
        constraints.constrain(self.0.size)
    }
    fn size(&self) -> Size {
        self.0.size
    }
    fn hit_test(&self, _: Point) -> bool {
        false
    }
    fn paint(&self, context: &mut PaintContext<'_>, origin: Point) -> bool {
        let font = font();
        for (row, line) in self.0.lines.iter().enumerate() {
            let y = origin.y + 12.0 + row as f32 * LINE_HEIGHT;
            context.draw_text_with_font_stack(
                Point::new(origin.x + 9.0, y),
                line.number
                    .map_or_else(|| "  >".into(), |number| format!("{number:3}")),
                Color::rgb(133u8, 151u8, 181u8),
                FONT_SIZE,
                font.clone(),
            );
            context.draw_text_with_font_stack(
                Point::new(origin.x + GUTTER, y),
                line.text.clone(),
                if line.text.trim_start().starts_with("//") {
                    Color::rgb(153u8, 190u8, 168u8)
                } else {
                    Color::rgb(234u8, 240u8, 251u8)
                },
                FONT_SIZE,
                font.clone(),
            );
        }
        true
    }
    fn render(&mut self) {}
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn code_wrap_preserves_unicode_and_original_line_numbers() {
        let text = CodeText::new(
            "let greeting = \"こんにちは\";\n\n// a comment",
            160.0,
            true,
        );
        assert!(text.lines.iter().any(|line| line.number.is_none()));
        assert_eq!(
            text.lines
                .iter()
                .filter_map(|line| line.number)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            text.lines
                .iter()
                .map(|line| line.text.as_str())
                .collect::<String>(),
            "let greeting = \"こんにちは\";// a comment"
        );
        assert!(text.size.width <= 161.0);
        assert!(
            CodeText::new("x".repeat(120).as_str(), 160.0, false)
                .size
                .width
                > 160.0
        );
    }
}
