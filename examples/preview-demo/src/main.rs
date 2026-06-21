use scarlet_ui::hstack;
use scarlet_ui::prelude::*;
use scarlet_ui::vstack;
use scarlet_ui::{Icon, NavigationLink};

#[derive(Clone)]
struct PreviewApp {
    count: State<i32>,
}

impl Default for PreviewApp {
    fn default() -> Self {
        Self {
            count: State::initial(StateId::new(0)),
        }
    }
}

impl PreviewApp {
    fn content(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("ScarletUI Preview").font_size(28.0),
            Text::new(format!("Count: {}", self.count.get())).font_size(20.0),
            Button::new("Increment").on_click({
                let count = self.count.clone();
                move || count.set(count.get() + 1)
            }),
        }
        .spacing(12.0)
        .padding(20.0)
    }
}

impl View for PreviewApp {
    fn create_element(&self) -> Box<dyn Element> {
        self.content().create_element()
    }

    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![&self.count as &dyn Listenable]
    }

    fn as_any(&self) -> &dyn core::any::Any {
        self
    }
}

impl Application for PreviewApp {
    fn scenes(&self) -> impl Scene {
        WindowGroup::new(
            "main",
            Window::new("Preview Demo", self.content()).size(Size::new(420.0, 260.0)),
        )
    }
}

#[derive(Clone)]
struct WidgetFactory {
    slider_value: State<f32>,
    toggle_on: State<bool>,
    text_value: State<String>,
    selected: State<usize>,
    text_document: State<TextDocument>,
    text_selection: State<TextSelection>,
}

impl Default for WidgetFactory {
    fn default() -> Self {
        Self {
            slider_value: State::new(StateId::new(20), 0.42),
            toggle_on: State::new(StateId::new(21), true),
            text_value: State::new(StateId::new(22), String::from("Factory text field")),
            selected: State::new(StateId::new(23), 1usize),
            text_document: State::new(
                StateId::new(24),
                TextDocument::from_str(
                    "# Hello TextView\n\nType here...\n\n- Item 1\n- Item 2\n- Item 3\n",
                ),
            ),
            text_selection: State::new(StateId::new(25), TextSelection::collapsed(0)),
        }
    }
}

impl WidgetFactory {
    fn row<V: View + Clone>(&self, name: &str, control: V) -> impl View + Clone + use<V> {
        hstack! {
            Text::new(name.to_owned()).font_size(14.0).frame(150.0, 32.0),
            control,
        }
        .spacing(18.0)
    }

    fn button(&self) -> impl View + Clone + use<> {
        Button::new("Factory Button").padding(12.0)
    }

    fn text_field(&self) -> impl View + Clone + use<> {
        TextField::new(self.text_value.clone()).placeholder("Enter value")
    }

    fn slider(&self) -> impl View + Clone + use<> {
        Slider::new(self.slider_value.clone()).min(0.0).max(1.0)
    }

    fn toggle(&self) -> impl View + Clone + use<> {
        Toggle::new(self.toggle_on.clone())
    }

    fn progress(&self) -> impl View + Clone + use<> {
        ProgressView::new(0.68)
    }

    fn select(&self) -> impl View + Clone + use<> {
        Select::new(
            vec![
                String::from("Compact"),
                String::from("Regular"),
                String::from("Expanded"),
            ],
            self.selected.clone(),
        )
    }

    fn rectangle(&self) -> impl View + Clone + use<> {
        Rectangle::new()
            .fill(Color::rgb(235u8, 242u8, 255u8))
            .corner_radius(8.0)
            .border(1.0, Color::rgb(105u8, 135u8, 210u8))
            .frame(220.0, 28.0)
    }

    fn divider(&self) -> impl View + Clone + use<> {
        Divider::new().frame(220.0, 1.0)
    }

    fn text_view(&self) -> impl View + Clone + use<> {
        TextView::with_document(self.text_document.clone(), self.text_selection.clone())
            .placeholder("Type something...")
            .font_size(14.0)
            .padding(8.0)
            .frame_height(200.0)
    }

    fn overview_page(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("Widget Factory").font_size(28.0),
            Text::new("PaintCommand default rendering").font_size(15.0),
            self.row("ProgressView", self.progress()),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
        }
        .spacing(16.0)
        .padding(24.0)
    }

    fn controls_page(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("Controls").font_size(24.0),
            self.row("Button", self.button()),
            self.row("Toggle", self.toggle()),
            self.row("Slider", self.slider()),
            self.row("ProgressView", self.progress()),
        }
        .spacing(16.0)
        .padding(24.0)
    }

    fn inputs_page(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("Inputs").font_size(24.0),
            self.row("TextField", self.text_field()),
            self.row("Select", self.select()),
            self.row("Slider", self.slider()),
            Text::new("TextView (multi-line editor)").font_size(14.0),
            self.text_view(),
        }
        .spacing(16.0)
        .padding(24.0)
    }

    fn display_page(&self) -> impl View + Clone + use<> {
        vstack! {
            Text::new("Display").font_size(24.0),
            self.row("Text", Text::new("Factory text sample").font_size(16.0)),
            self.row("Rectangle", self.rectangle()),
            self.row("Divider", self.divider()),
            self.row("ProgressView", self.progress()),
        }
        .spacing(16.0)
        .padding(24.0)
    }
}

#[scarlet_ui::preview(width = 420.0, height = 260.0)]
fn counter_preview() -> impl View + Clone {
    PreviewApp::default()
}

#[scarlet_ui::preview(width = 320.0, height = 180.0)]
fn button_preview() -> impl View + Clone {
    Button::new("Standalone Button reloads").padding(20.0)
}

#[scarlet_ui::preview(width = 520.0, height = 520.0)]
fn showcase_preview() -> impl View + Clone {
    let slider_value = State::new(StateId::new(10), 0.35f32);
    let toggle_on = State::new(StateId::new(11), true);
    let text_value = State::new(StateId::new(12), String::from("Hello ScarletUI"));
    let selected = State::new(StateId::new(13), 1usize);

    vstack! {
        Text::new("Showcase").font_size(28.0),
        TextField::new(text_value).placeholder("Type something"),
        Slider::new(slider_value.clone()).min(0.0).max(1.0),
        Toggle::new(toggle_on),
        ProgressView::new(0.62),
        Select::new(
            vec![String::from("Alpha"), String::from("Beta"), String::from("Gamma")],
            selected,
        ),
        Button::new("Primary Action").padding(12.0),
    }
    .spacing(16.0)
    .padding(20.0)
}

#[scarlet_ui::preview(width = 860.0, height = 560.0)]
fn widget_factory_preview() -> impl View + Clone {
    let factory = WidgetFactory::default();
    let overview = factory.clone();
    let controls = factory.clone();
    let inputs = factory.clone();
    let display = factory.clone();

    scarlet_ui::navigation! {
        NavigationLink::new("Overview", Icon::Home, move || overview.overview_page()),
        NavigationLink::new("Controls", Icon::Settings, move || controls.controls_page()),
        NavigationLink::new("Inputs", Icon::Search, move || inputs.inputs_page()),
        NavigationLink::new("Display", Icon::Info, move || display.display_page()),
    }
    .sidebar_width(190.0)
}

#[scarlet_ui::preview(width = 600.0, height = 400.0)]
fn text_view_preview() -> impl View + Clone {
    let text = State::new(
        StateId::new(30),
        String::from(
            "# TextView Demo\n\nThis is a multi-line text editor built with ScarletUI.\n\nFeatures:\n- Keyboard editing with cursor movement\n- Japanese IME support (preedit/commit)\n- Mouse click/drag selection\n- Double-click word select, triple-click line select\n- Both wrap modes (None / Soft)\n- Horizontal and vertical scrolling\n- Clipboard callbacks (copy/paste/cut)\n\nTry typing here!\n",
        ),
    );
    let selection = State::new(StateId::new(31), TextSelection::collapsed(0));
    TextView::new(text, selection)
        .placeholder("Start typing...")
        .font_size(14.0)
        .padding(12.0)
}

fn main() -> scarlet_ui::Result<()> {
    let mut app = PreviewApp::default();
    app.run()
}
