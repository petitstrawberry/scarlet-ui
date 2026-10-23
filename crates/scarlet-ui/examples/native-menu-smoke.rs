//! Two-window native-menu probe. Actions only change counters; no files are read/written.
//! Cmd+I must increment only the keyboard counter; menu clicks only the native counter.
use scarlet_ui::{
    event::{KeyCode, KeyEvent},
    prelude::*,
};
use std::sync::Arc;

#[derive(Clone)]
struct Probe {
    counts: [State<usize>; 2],
    keyboard_actions: [State<usize>; 2],
    enabled: State<bool>,
    text: [State<String>; 2],
    close_requests: State<usize>,
    allow_close: State<bool>,
}

impl Probe {
    fn window(&self, index: usize) -> impl Scene {
        let count = self.counts[index].clone();
        let enabled = self.enabled.clone();
        let keyboard_actions = self.keyboard_actions[index].clone();
        let owner = if index == 0 { "A" } else { "B" };
        let action: scarlet_ui::menu_model::MenuCallback = Arc::new(move || {
            count.update(|n| *n += 1);
            println!("native {owner}: {}", count.get());
        });
        let menu = MenuBarModel::new(vec![
            MenuItemModel::app().children(vec![MenuEntry::Item(
                MenuItemModel::new("preferences", "Probe Preferences…").on_activate(action.clone()),
            )]),
            MenuItemModel::new("file", "File").children(vec![
                MenuEntry::Item(
                    MenuItemModel::new("other", "Open Other Window").on_activate(Arc::new(
                        move || {
                            scarlet_ui::open_window(if index == 0 { "menu-B" } else { "menu-A" });
                        },
                    )),
                ),
                MenuEntry::Item(
                    MenuItemModel::new(
                        "activate",
                        format!("Activate {owner} ({})", self.counts[index].get()),
                    )
                    .shortcut("Cmd+I")
                    .enabled(self.enabled.get())
                    .on_activate(action.clone()),
                ),
                MenuEntry::Item(
                    MenuItemModel::new("shift", "Shift Action")
                        .shortcut("Cmd+Shift+I")
                        .on_activate(action.clone()),
                ),
                MenuEntry::Item(
                    MenuItemModel::new("function", "Function Action")
                        .shortcut("F12")
                        .on_activate(action.clone()),
                ),
                MenuEntry::Item(
                    MenuItemModel::new("control", "Control Action")
                        .shortcut("Ctrl+J")
                        .on_activate(action.clone()),
                ),
                MenuEntry::Separator,
                MenuEntry::Item(
                    MenuItemModel::new("disabled", "Disabled Action")
                        .enabled(false)
                        .on_activate(action.clone()),
                ),
                MenuEntry::Item(
                    MenuItemModel::new("nested", "Nested").children(vec![MenuEntry::Item(
                        MenuItemModel::new("nested-action", "Nested Action")
                            .on_activate(action.clone()),
                    )]),
                ),
            ]),
            MenuItemModel::new("edit", "Edit").children(
                [
                    ("select-all", "Select All", "Cmd+A"),
                    ("cut", "Cut", "Cmd+X"),
                    ("copy", "Copy", "Cmd+C"),
                    ("paste", "Paste", "Cmd+V"),
                    ("undo", "Undo", "Cmd+Z"),
                ]
                .into_iter()
                .map(|(id, title, key)| {
                    MenuEntry::Item(
                        MenuItemModel::new(id, title)
                            .shortcut(key)
                            .on_activate(action.clone()),
                    )
                })
                .collect(),
            ),
            MenuItemModel::new("view", "View").children(vec![
                MenuEntry::Item(
                    MenuItemModel::new("enabled", "Enable Action")
                        .checked(self.enabled.get())
                        .shortcut("Cmd+E")
                        .on_activate(Arc::new(move || {
                            enabled.update(|value| *value = !*value);
                        })),
                ),
                MenuEntry::Item(
                    MenuItemModel::new("chord", "Application Chord").shortcut("Cmd+K Cmd+T"),
                ),
            ]),
        ]);
        let key_enabled = self.enabled.clone();
        let allow_close = self.allow_close.clone();
        let content = scarlet_ui::vstack! {
            Text::new(format!("Window {owner} — native actions: {}", self.counts[index].get())),
            Text::new(format!("Keyboard actions: {}", self.keyboard_actions[index].get())),
            Text::new(format!("Close requests: {} (allowed: {})", self.close_requests.get(), self.allow_close.get())),
            Button::new("Allow closing probe").on_click(move || allow_close.set(true)),
            Text::new("Cmd+I / Shift+Cmd+I / F12 / Ctrl+J: one action"),
            Text::new("Cmd+E: toggle checked/enabled. Switch windows to verify ownership."),
            TextField::new(self.text[index].clone()).placeholder("Dark placeholder probe")
                .placeholder_color(Color::rgb(154u8, 160u8, 171u8))
                .text_color(Color::rgb(213u8, 216u8, 222u8))
                .background_color(Color::rgb(36u8, 40u8, 47u8)).autofocus(true).frame(300.0, 32.0),
        }.spacing(14.0).padding(20.0).on_key(move |event| {
            if let KeyEvent::Pressed { keycode, modifiers } = event {
                if (matches!(keycode, KeyCode::Char('i' | 'I')) && modifiers.super_key)
                    || keycode == KeyCode::F(12) || (matches!(keycode, KeyCode::Char('j' | 'J')) && modifiers.control) {
                    keyboard_actions.update(|n| *n += 1);
                    println!("keyboard {owner}: {keycode:?}");
                    return true;
                }
                if matches!(keycode, KeyCode::Char('e' | 'E')) && modifiers.super_key {
                    key_enabled.update(|value| *value = !*value);
                    return true;
                }
            }
            false
        });
        Window::new(format!("ScarletUI Native Menu {owner}"), content)
            .scene_key(format!("menu-{owner}"))
            .size(Size::new(580.0, 350.0))
            .menu_bar(menu)
            .placement(scarlet_ui::platform::WindowPlacement::At {
                x: 100 + index as i32 * 620,
                y: 150,
            })
    }
}

impl View for Probe {
    fn create_element(&self) -> Box<dyn Element> {
        Text::new("Menu probe").create_element()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn listenables(&self) -> Vec<&dyn Listenable> {
        vec![
            &self.counts[0],
            &self.counts[1],
            &self.keyboard_actions[0],
            &self.keyboard_actions[1],
            &self.enabled,
            &self.close_requests,
            &self.allow_close,
        ]
    }
}

impl Application for Probe {
    fn scenes(&self) -> impl Scene {
        (self.window(0), self.window(1))
    }
    fn on_window_close_requested(&mut self, _: &scarlet_ui::WindowContext) -> bool {
        self.close_requests.update(|count| *count += 1);
        self.allow_close.get()
    }
}

fn main() {
    let mut probe = Probe {
        close_requests: State::new(StateId::new(8107), 0),
        allow_close: State::new(StateId::new(8108), false),
        counts: [
            State::new(StateId::new(8100), 0),
            State::new(StateId::new(8101), 0),
        ],
        keyboard_actions: [
            State::new(StateId::new(8102), 0),
            State::new(StateId::new(8103), 0),
        ],
        enabled: State::new(StateId::new(8104), true),
        text: [
            State::new(StateId::new(8105), String::new()),
            State::new(StateId::new(8106), String::new()),
        ],
    };
    probe.run().unwrap();
}
