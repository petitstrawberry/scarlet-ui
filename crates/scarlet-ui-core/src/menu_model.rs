//! Menu model definitions for application menu bars.

use crate::os::{Mutex, spawn_detached};
use crate::views::{MenuBar, MenuItem};
use alloc::collections::{BTreeMap, BTreeSet};
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

/// Menu activation callback.
///
/// Callbacks execute on a detached worker rather than the application event
/// loop. The `Send + Sync` contract permits that execution model and prevents
/// blocking IPC or filesystem work from freezing every window in the process.
pub type MenuCallback = Arc<dyn Fn() + Send + Sync + 'static>;

#[derive(Clone)]
pub enum MenuEntry {
    Item(MenuItemModel),
    Separator,
}

#[derive(Clone)]
pub struct MenuItemModel {
    id: String,
    title: String,
    enabled: bool,
    shortcut: Option<String>,
    children: Vec<MenuEntry>,
    on_activate: Option<MenuCallback>,
}

impl MenuItemModel {
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            enabled: true,
            shortcut: None,
            children: Vec::new(),
            on_activate: None,
        }
    }

    /// Create an app-menu item. The taskbar merges its children into the
    /// auto-generated app-name dropdown.
    pub fn app() -> Self {
        Self::new("__app__", "")
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<String>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn children(mut self, children: Vec<MenuEntry>) -> Self {
        self.children = children;
        self
    }

    pub fn on_activate(mut self, callback: MenuCallback) -> Self {
        self.on_activate = Some(callback);
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn enabled_value(&self) -> bool {
        self.enabled
    }

    pub fn shortcut_value(&self) -> Option<&str> {
        self.shortcut.as_deref()
    }

    pub fn children_value(&self) -> &[MenuEntry] {
        &self.children
    }
}

#[derive(Clone)]
pub struct MenuBarModel {
    items: Vec<MenuItemModel>,
}

impl MenuBarModel {
    pub fn new(items: Vec<MenuItemModel>) -> Self {
        Self { items }
    }

    pub fn items(&self) -> &[MenuItemModel] {
        &self.items
    }

    pub fn menu_titles(&self) -> String {
        let mut out = String::new();
        for (idx, item) in self.items.iter().enumerate() {
            if idx > 0 {
                out.push('|');
            }
            out.push_str(item.title());
        }
        out
    }

    pub fn to_json(&self) -> String {
        let mut out = String::from("{\"items\":[");
        for (idx, item) in self.items.iter().enumerate() {
            if idx > 0 {
                out.push(',');
            }
            write_menu_item_json(&mut out, item);
        }
        out.push_str("]}");
        out
    }

    pub fn to_menu_bar_view(&self) -> MenuBar {
        let items = self
            .items
            .iter()
            .map(|item| MenuItem::new(item.title()))
            .collect();
        MenuBar::new(items)
    }
}

impl Default for MenuBarModel {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

fn write_menu_item_json(out: &mut String, item: &MenuItemModel) {
    out.push('{');
    out.push_str("\"id\":\"");
    push_json_string(out, item.id());
    out.push_str("\",\"title\":\"");
    push_json_string(out, item.title());
    out.push_str("\",\"enabled\":");
    out.push_str(if item.enabled_value() {
        "true"
    } else {
        "false"
    });
    out.push_str(",\"shortcut\":");
    if let Some(sc) = item.shortcut_value() {
        out.push('"');
        push_json_string(out, sc);
        out.push('"');
    } else {
        out.push_str("null");
    }
    out.push_str(",\"items\":[");
    for (idx, entry) in item.children_value().iter().enumerate() {
        if idx > 0 {
            out.push(',');
        }
        match entry {
            MenuEntry::Separator => out.push_str("{\"separator\":true}"),
            MenuEntry::Item(child) => write_menu_item_json(out, child),
        }
    }
    out.push_str("]}");
}

fn push_json_string(out: &mut String, value: &str) {
    // JSON strings are UTF-8, so escaping must operate on Unicode scalar
    // values rather than individual bytes. Casting UTF-8 continuation bytes
    // to `char` corrupts titles such as `Open…` and other non-ASCII text.
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => {
                let _ = alloc::fmt::write(out, format_args!("\\u{:04x}", ch as u32));
            }
            ch => out.push(ch),
        }
    }
}

type MenuCallbackKey = (u32, String);

static MENU_CALLBACKS: Mutex<BTreeMap<MenuCallbackKey, MenuCallback>> = Mutex::new(BTreeMap::new());
static MENU_CALLBACKS_IN_FLIGHT: Mutex<BTreeSet<MenuCallbackKey>> =
    Mutex::new(BTreeSet::new());

struct MenuCallbackCompletion {
    key: MenuCallbackKey,
}

impl Drop for MenuCallbackCompletion {
    fn drop(&mut self) {
        MENU_CALLBACKS_IN_FLIGHT.lock().remove(&self.key);
    }
}

pub fn register_menu_callbacks(window_id: u32, menu_bar: &MenuBarModel) {
    let mut registry = MENU_CALLBACKS.lock();
    registry.retain(|(id, _), _| *id != window_id);
    for item in menu_bar.items() {
        collect_callbacks(window_id, item, &mut registry);
    }
}

/// Invoke a menu callback without blocking the application event loop.
///
/// At most one invocation of a given `(window, item)` callback may run at a
/// time. Repeated activation while a slow callback is still performing IPC is
/// treated as handled but does not start duplicate work.
pub fn invoke_menu_callback(window_id: u32, item_id: &str) -> bool {
    let key = (window_id, item_id.to_string());
    let callback = MENU_CALLBACKS.lock().get(&key).cloned();
    let Some(callback) = callback else {
        return false;
    };

    if !MENU_CALLBACKS_IN_FLIGHT.lock().insert(key.clone()) {
        return true;
    }

    spawn_detached(move || {
        let _completion = MenuCallbackCompletion { key };
        callback();
    });
    true
}

pub fn unregister_menu_callbacks(window_id: u32) {
    MENU_CALLBACKS
        .lock()
        .retain(|(registered_window_id, _), _| *registered_window_id != window_id);
}

fn collect_callbacks(
    window_id: u32,
    item: &MenuItemModel,
    registry: &mut BTreeMap<MenuCallbackKey, MenuCallback>,
) {
    if let Some(callback) = &item.on_activate {
        registry.insert((window_id, item.id().to_string()), callback.clone());
    }
    for entry in item.children_value() {
        if let MenuEntry::Item(child) = entry {
            collect_callbacks(window_id, child, registry);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MenuBarModel, MenuEntry, MenuItemModel, invoke_menu_callback, register_menu_callbacks,
        unregister_menu_callbacks,
    };
    use alloc::sync::Arc;
    use alloc::vec;
    use core::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn menu_json_preserves_unicode_text() {
        let json = MenuBarModel::new(vec![MenuItemModel::new("file", "File").children(vec![
            MenuEntry::Item(MenuItemModel::new("open", "Open…").shortcut("Ctrl+O")),
        ])])
        .to_json();

        assert!(json.contains("\"title\":\"Open…\""));
        assert!(json.contains("\"shortcut\":\"Ctrl+O\""));
    }

    #[test]
    fn slow_menu_callback_runs_off_thread_and_is_not_reentered() {
        let window_id = 0xfeed;
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_callback = Arc::clone(&calls);
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let callback = Arc::new(move || {
            calls_for_callback.fetch_add(1, Ordering::AcqRel);
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        let menu = MenuBarModel::new(vec![
            MenuItemModel::new("file", "File").children(vec![MenuEntry::Item(
                MenuItemModel::new("open", "Open").on_activate(callback),
            )]),
        ]);
        register_menu_callbacks(window_id, &menu);

        assert!(invoke_menu_callback(window_id, "open"));
        started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(invoke_menu_callback(window_id, "open"));
        assert_eq!(calls.load(Ordering::Acquire), 1);

        release_tx.send(()).unwrap();
        for _ in 0..100 {
            if calls.load(Ordering::Acquire) == 1 {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        unregister_menu_callbacks(window_id);
    }
}
