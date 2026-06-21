use scarlet_ui::hstack;
use scarlet_ui::prelude::*;
use scarlet_ui::vstack;

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
}

impl Default for WidgetFactory {
    fn default() -> Self {
        Self {
            slider_value: State::new(StateId::new(20), 0.42),
            toggle_on: State::new(StateId::new(21), true),
            text_value: State::new(StateId::new(22), String::from("Factory text field")),
            selected: State::new(StateId::new(23), 1usize),
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

#[scarlet_ui::preview(width = 760.0, height = 620.0)]
fn widget_factory_preview() -> impl View + Clone {
    let factory = WidgetFactory::default();

    vstack! {
        Text::new("Widget Factory").font_size(28.0),
        factory.row("Button", factory.button()),
        factory.row("TextField", factory.text_field()),
        factory.row("Slider", factory.slider()),
        factory.row("Toggle", factory.toggle()),
        factory.row("ProgressView", factory.progress()),
        factory.row("Select", factory.select()),
        factory.row("Rectangle", factory.rectangle()),
        factory.row("Divider", factory.divider()),
    }
    .spacing(14.0)
    .padding(24.0)
}

fn main() -> scarlet_ui::Result<()> {
    let mut app = PreviewApp::default();
    app.run()
}
