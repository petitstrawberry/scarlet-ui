//! App-defined key equivalents are display hints; keyboard dispatch stays in winit.
//! All objects in this module are created, updated and dropped on the UI thread.

use super::WinitEventState;
use objc2::rc::{Retained, WeakId};
use objc2::{ClassType, DeclaredClass, declare_class, msg_send, msg_send_id, mutability, sel};
use objc2_app_kit::{
    NSApplication, NSEvent, NSEventModifierFlags, NSMenu, NSMenuItem, NSView, NSWindow,
};
use objc2_foundation::{MainThreadMarker, NSCopying, NSObject, NSObjectProtocol, NSString};
use scarlet_ui_core::{
    error::{Error, Result},
    menu_model::invoke_menu_callback,
};
use serde::Deserialize;
use std::{
    cell::RefCell,
    collections::BTreeMap,
    rc::{Rc, Weak},
};
use winit::{
    raw_window_handle::{HasWindowHandle, RawWindowHandle},
    window::{Window, WindowId},
};

#[derive(Clone, Debug, Default, Deserialize)]
struct MenuSpec {
    #[serde(default)]
    items: Vec<ItemSpec>,
}

#[derive(Clone, Debug, Deserialize)]
struct ItemSpec {
    #[serde(default)]
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default = "enabled_by_default")]
    enabled: bool,
    #[serde(default)]
    checked: bool,
    #[serde(default)]
    shortcut: Option<String>,
    #[serde(default)]
    items: Vec<ItemSpec>,
    #[serde(default)]
    separator: bool,
}

fn enabled_by_default() -> bool {
    true
}

fn parse_menu(json: &str) -> Result<MenuSpec> {
    if json.is_empty() {
        return Ok(MenuSpec::default());
    }
    serde_json::from_str(json).map_err(|_| Error::EventDispatchError)
}

#[derive(Debug, PartialEq)]
struct Shortcut {
    key: String,
    modifiers: NSEventModifierFlags,
}

/// A chord is deliberately not registered as its first stroke: doing so would
/// swallow the editor's chord prefix. Applications continue handling chords.
fn parse_shortcut(text: &str) -> Option<Shortcut> {
    let parts: Vec<_> = text.split('+').map(str::trim).collect();
    let (key, modifiers) = parts.split_last()?;
    let mut flags = NSEventModifierFlags::empty();
    for modifier in modifiers {
        flags |= match modifier.to_ascii_lowercase().as_str() {
            "cmd" | "command" | "super" | "meta" | "primary" => {
                NSEventModifierFlags::NSEventModifierFlagCommand
            }
            "ctrl" | "control" => NSEventModifierFlags::NSEventModifierFlagControl,
            "alt" | "option" | "opt" => NSEventModifierFlags::NSEventModifierFlagOption,
            "shift" => NSEventModifierFlags::NSEventModifierFlagShift,
            _ => return None,
        };
    }
    let normalized = key.to_ascii_lowercase();
    let key = match normalized.as_str() {
        "space" => " ".into(),
        "enter" | "return" => "\r".into(),
        "tab" => "\t".into(),
        "escape" | "esc" => "\u{1b}".into(),
        "backspace" => "\u{8}".into(),
        "delete" | "del" => "\u{f728}".into(),
        "up" => "\u{f700}".into(),
        "down" => "\u{f701}".into(),
        "left" => "\u{f702}".into(),
        "right" => "\u{f703}".into(),
        "home" => "\u{f729}".into(),
        "end" => "\u{f72b}".into(),
        "pageup" => "\u{f72c}".into(),
        "pagedown" => "\u{f72d}".into(),
        key if key.starts_with('f') && key.len() > 1 => {
            let number: u32 = key[1..].parse().ok()?;
            if !(1..=35).contains(&number) {
                return None;
            }
            char::from_u32(0xf704 + number - 1)?.to_string()
        }
        key if key.chars().count() == 1 => key.to_owned(),
        _ => return None,
    };
    Some(Shortcut {
        key,
        modifiers: flags,
    })
}

type CloseTargets = Rc<RefCell<Vec<Weak<RefCell<WinitEventState>>>>>;

enum MenuAction {
    Callback(String),
    CloseWindows(CloseTargets),
}

struct ActionState {
    surface_id: u32,
    owner: Option<WeakId<NSWindow>>,
    action: MenuAction,
    enabled: bool,
    // Retired menu items may be retained by AppKit during tracking/accessibility.
    // A weak lifetime prevents them from invoking a replacement model's action.
    lifetime: Weak<()>,
}

declare_class!(
    struct ScarletMenuAction;
    unsafe impl ClassType for ScarletMenuAction {
        type Super = NSObject;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "ScarletUIMenuAction";
    }
    impl DeclaredClass for ScarletMenuAction { type Ivars = ActionState; }
    unsafe impl NSObjectProtocol for ScarletMenuAction {}
    unsafe impl ScarletMenuAction {
        #[method(activate:)]
        fn activate(&self, _sender: &NSMenuItem) {
            let state = self.ivars();
            if !state.enabled || state.lifetime.upgrade().is_none() { return; }
            if let Some(expected) = &state.owner {
                let app = NSApplication::sharedApplication(MainThreadMarker::from(self));
                let Some(owner) = app.keyWindow() else { return; };
                // Check actual key-window identity, including before queued
                // winit focus events have caught up. Sheets are not their owner.
                if expected.load().as_deref() != Some(&owner)
                    || unsafe { owner.attachedSheet() }.is_some() { return; }
            }
            match &state.action {
                MenuAction::Callback(id) => { invoke_menu_callback(state.surface_id, id); }
                MenuAction::CloseWindows(windows) => {
                    for window in windows.borrow().iter().filter_map(Weak::upgrade) {
                        window.borrow_mut().push(scarlet_ui_core::event::Event::Window(
                            scarlet_ui_core::event::WindowEvent::CloseRequested,
                        ));
                    }
                }
            }
        }
    }
);

struct WindowMenu {
    surface_id: u32,
    owner: WeakId<NSWindow>,
    events: Weak<RefCell<WinitEventState>>,
    spec: MenuSpec,
}

struct InstalledMenu {
    owner: WindowId,
    menu: Retained<NSMenu>,
    _lifetime: Rc<()>,
}

#[derive(Default)]
pub(crate) struct NativeMenus {
    windows: BTreeMap<WindowId, WindowMenu>,
    base: Option<Retained<NSMenu>>,
    original: Option<Retained<NSMenu>>,
    close_targets: CloseTargets,
    quit_lifetime: Rc<()>,
    installed: Option<InstalledMenu>,
    dirty: bool,
    launched: bool,
}

impl NativeMenus {
    pub(crate) fn launched(&mut self) {
        self.launched = true;
        self.refresh();
    }

    pub(crate) fn update(
        &mut self,
        window: &Window,
        surface_id: u32,
        json: &str,
        events: Weak<RefCell<WinitEventState>>,
    ) -> Result<()> {
        // Parse before changing live state, so a failed publish can be retried.
        let spec = parse_menu(json)?;
        let handle = window
            .window_handle()
            .map_err(|_| Error::EventDispatchError)?;
        let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
            return Err(Error::EventDispatchError);
        };
        // SAFETY: winit owns this view and the caller runs on its UI thread.
        let view = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
        let owner = view.window().ok_or(Error::EventDispatchError)?;
        self.windows.insert(
            window.id(),
            WindowMenu {
                surface_id,
                owner: WeakId::new(&owner),
                events,
                spec,
            },
        );
        *self.close_targets.borrow_mut() = self
            .windows
            .values()
            .map(|menu| menu.events.clone())
            .collect();
        self.dirty = true;
        self.refresh();
        Ok(())
    }

    pub(crate) fn remove(&mut self, window: WindowId) {
        self.windows.remove(&window);
        *self.close_targets.borrow_mut() = self
            .windows
            .values()
            .map(|menu| menu.events.clone())
            .collect();
        self.dirty = true;
        self.refresh();
    }

    pub(crate) fn refresh(&mut self) {
        // Before didFinishLaunching, AppKit may already have a provisional
        // menu that winit will replace. Never cache/install against that menu.
        if !self.launched {
            return;
        }
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let app = NSApplication::sharedApplication(mtm);
        // Winit installs its standard About/Services/Hide/Quit menu at launch.
        // Capture it lazily, after launch, and preserve it across model changes.
        if self.base.is_none() {
            if let Some(original) = unsafe { app.mainMenu() } {
                let base = original.copy();
                // Even with all windows minimized (or a sheet key), Quit must
                // use close callbacks, not the default unconditional terminate.
                if let Some(item) = standard_quit_item(&base) {
                    attach_target(
                        mtm,
                        &item,
                        ActionState {
                            surface_id: 0,
                            owner: None,
                            action: MenuAction::CloseWindows(self.close_targets.clone()),
                            enabled: true,
                            lifetime: Rc::downgrade(&self.quit_lifetime),
                        },
                    );
                }
                self.original = Some(original);
                self.base = Some(base);
            }
        }
        let Some(base) = &self.base else {
            return;
        };
        let key_window = app.keyWindow();
        let focused = self
            .windows
            .iter()
            .find(|(_, menu)| key_window.is_some() && menu.owner.load() == key_window);
        let owner = focused.map(|(id, _)| *id);
        if !self.dirty && self.installed.as_ref().map(|menu| menu.owner) == owner {
            return;
        }
        self.dirty = false;
        if let Some((id, model)) = focused {
            let lifetime = Rc::new(());
            let menu = build_menu(mtm, base, model, &lifetime, self.close_targets.clone());
            app.setMainMenu(Some(&menu));
            self.installed = Some(InstalledMenu {
                owner: *id,
                menu,
                _lifetime: lifetime,
            });
        } else {
            app.setMainMenu(Some(base));
            self.installed = None;
        }
    }
}

impl Drop for NativeMenus {
    fn drop(&mut self) {
        if let (Some(mtm), Some(original)) = (MainThreadMarker::new(), &self.original) {
            let app = NSApplication::sharedApplication(mtm);
            let current = unsafe { app.mainMenu() };
            if current.as_deref() == self.base.as_deref()
                || current.as_deref() == self.installed.as_ref().map(|menu| &*menu.menu)
            {
                app.setMainMenu(Some(original));
            }
        }
    }
}

fn standard_quit_item(menu: &NSMenu) -> Option<Retained<NSMenuItem>> {
    let app_menu = unsafe { menu.itemAtIndex(0)?.submenu()? };
    (0..unsafe { app_menu.numberOfItems() }).filter_map(|index| unsafe { app_menu.itemAtIndex(index) })
        .find(|item| unsafe { item.keyEquivalent().to_string() == "q" }
            && unsafe { item.keyEquivalentModifierMask() } == NSEventModifierFlags::NSEventModifierFlagCommand)
}

fn build_menu(
    mtm: MainThreadMarker,
    base: &NSMenu,
    window: &WindowMenu,
    lifetime: &Rc<()>,
    windows: CloseTargets,
) -> Retained<NSMenu> {
    let mut shortcuts = Vec::new();
    collect_shortcuts(&window.spec.items, &mut shortcuts);
    let root = mtm.alloc::<ScarletMenuBar>().set_ivars(shortcuts);
    let title = NSString::from_str("");
    let menu: Retained<ScarletMenuBar> =
        unsafe { msg_send_id![super(root), initWithTitle: &*title] };
    for index in 0..unsafe { base.numberOfItems() } {
        if let Some(item) = unsafe { base.itemAtIndex(index) } {
            menu.addItem(&item.copy());
        }
    }
    // AppKit's terminate: bypasses Application::on_window_close_requested and
    // would discard dirty documents. Prefer an app-defined Quit callback; the
    // fallback queues ordinary close requests, which applications can veto.
    if let Some(app_menu) =
        unsafe { menu.itemAtIndex(0) }.and_then(|item| unsafe { item.submenu() })
    {
        for index in 0..unsafe { app_menu.numberOfItems() } {
            if let Some(item) = unsafe { app_menu.itemAtIndex(index) } {
                if unsafe { item.keyEquivalent().to_string() == "q" }
                    && unsafe { item.keyEquivalentModifierMask() }
                        == NSEventModifierFlags::NSEventModifierFlagCommand
                {
                    let quit = find_quit(&window.spec.items, true);
                    let enabled = quit.map_or(true, |(_, enabled)| enabled);
                    let action = quit.map_or_else(
                        || MenuAction::CloseWindows(windows.clone()),
                        |(spec, _)| MenuAction::Callback(spec.id.clone()),
                    );
                    attach_action(mtm, &item, window, lifetime, enabled, action);
                    unsafe {
                        item.setEnabled(enabled);
                        app_menu.setAutoenablesItems(false);
                    }
                }
            }
        }
    }
    unsafe { menu.setAutoenablesItems(false) };
    for spec in &window.spec.items {
        if spec.id == "__app__" {
            if let Some(app_menu) =
                unsafe { menu.itemAtIndex(0) }.and_then(|item| unsafe { item.submenu() })
            {
                // Custom children use explicit model-enabled state. Standard
                // application actions and Services keep their native targets.
                unsafe { app_menu.setAutoenablesItems(false) };
                for (index, child) in spec.items.iter().enumerate() {
                    let item = build_item(mtm, child, window, lifetime, spec.enabled);
                    unsafe { app_menu.insertItem_atIndex(&item, index as isize) };
                }
                if !spec.items.is_empty() {
                    unsafe {
                        app_menu.insertItem_atIndex(
                            &NSMenuItem::separatorItem(mtm),
                            spec.items.len() as isize,
                        )
                    };
                }
            }
        } else {
            let item = build_item(mtm, spec, window, lifetime, true);
            menu.addItem(&item);
        }
    }
    Retained::into_super(menu)
}

fn build_item(
    mtm: MainThreadMarker,
    spec: &ItemSpec,
    window: &WindowMenu,
    lifetime: &Rc<()>,
    parent_enabled: bool,
) -> Retained<NSMenuItem> {
    if spec.separator {
        return NSMenuItem::separatorItem(mtm);
    }
    let shortcut = spec.shortcut.as_deref().and_then(parse_shortcut);
    let key = shortcut.as_ref().map_or("", |sc| sc.key.as_str());
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            mtm.alloc(),
            &NSString::from_str(&spec.title),
            None,
            &NSString::from_str(key),
        )
    };
    item.setKeyEquivalentModifierMask(
        shortcut
            .as_ref()
            .map_or(NSEventModifierFlags::empty(), |sc| sc.modifiers),
    );
    let enabled = parent_enabled && spec.enabled;
    unsafe {
        item.setEnabled(enabled);
        item.setState(if spec.checked { 1 } else { 0 });
    }
    if spec.items.is_empty() {
        attach_action(
            mtm,
            &item,
            window,
            lifetime,
            enabled,
            MenuAction::Callback(spec.id.clone()),
        );
    } else {
        let submenu = NSMenu::new(mtm);
        unsafe {
            submenu.setTitle(&NSString::from_str(&spec.title));
            submenu.setAutoenablesItems(false);
        }
        for child in &spec.items {
            submenu.addItem(&build_item(mtm, child, window, lifetime, enabled));
        }
        item.setSubmenu(Some(&submenu));
    }
    item
}

fn attach_action(
    mtm: MainThreadMarker,
    item: &NSMenuItem,
    window: &WindowMenu,
    lifetime: &Rc<()>,
    enabled: bool,
    action: MenuAction,
) {
    attach_target(
        mtm,
        item,
        ActionState {
            surface_id: window.surface_id,
            owner: Some(window.owner.clone()),
            action,
            enabled,
            lifetime: Rc::downgrade(lifetime),
        },
    );
}

fn attach_target(mtm: MainThreadMarker, item: &NSMenuItem, state: ActionState) {
    let target = mtm.alloc::<ScarletMenuAction>().set_ivars(state);
    let target: Retained<ScarletMenuAction> = unsafe { msg_send_id![super(target), init] };
    unsafe {
        item.setAction(Some(sel!(activate:)));
        item.setTarget(Some(&target));
        // NSMenuItem.target is unretained; representedObject retains it even
        // if AppKit keeps the item around after the menu has been replaced.
        item.setRepresentedObject(Some(&target));
    }
}

fn collect_shortcuts(items: &[ItemSpec], out: &mut Vec<Shortcut>) {
    for item in items {
        if let Some(shortcut) = item.shortcut.as_deref().and_then(parse_shortcut) {
            out.push(shortcut);
        }
        collect_shortcuts(&item.items, out);
    }
}

fn find_quit(items: &[ItemSpec], parent_enabled: bool) -> Option<(&ItemSpec, bool)> {
    for item in items {
        let enabled = parent_enabled && item.enabled;
        if item
            .shortcut
            .as_deref()
            .and_then(parse_shortcut)
            .is_some_and(|sc| {
                sc.key == "q" && sc.modifiers == NSEventModifierFlags::NSEventModifierFlagCommand
            })
        {
            return Some((item, enabled));
        }
        if let Some(found) = find_quit(&item.items, enabled) {
            return Some(found);
        }
    }
    None
}

fn is_model_shortcut(shortcuts: &[Shortcut], key: &str, flags: NSEventModifierFlags) -> bool {
    let relevant = NSEventModifierFlags::NSEventModifierFlagCommand
        | NSEventModifierFlags::NSEventModifierFlagControl
        | NSEventModifierFlags::NSEventModifierFlagOption
        | NSEventModifierFlags::NSEventModifierFlagShift;
    shortcuts
        .iter()
        .any(|sc| sc.key == key.to_lowercase() && sc.modifiers == flags & relevant)
}

// Refuse native execution of app-defined keyboard equivalents, while leaving
// their normal macOS shortcut labels visible. This preserves focused TextField
// cut/copy/paste/select-all/undo and the app's existing shortcut/chord routing.
// Mouse menu activation still goes through ScarletMenuAction.
declare_class!(
    struct ScarletMenuBar;
    unsafe impl ClassType for ScarletMenuBar {
        #[inherits(NSObject)]
        type Super = NSMenu;
        type Mutability = mutability::MainThreadOnly;
        const NAME: &'static str = "ScarletUIMenuBar";
    }
    impl DeclaredClass for ScarletMenuBar { type Ivars = Vec<Shortcut>; }
    unsafe impl NSObjectProtocol for ScarletMenuBar {}
    unsafe impl ScarletMenuBar {
        #[method(performKeyEquivalent:)]
        fn perform_key_equivalent(&self, event: &NSEvent) -> bool {
            let characters = unsafe { event.charactersIgnoringModifiers() };
            if characters.is_some_and(|key| is_model_shortcut(self.ivars(), &key.to_string(), unsafe { event.modifierFlags() })) {
                false
            } else {
                unsafe { msg_send![super(self), performKeyEquivalent: event] }
            }
        }
    }
);

#[cfg(test)]
mod tests {
    use super::*;
    use scarlet_ui_core::{MenuBarModel, MenuEntry, MenuItemModel};

    #[test]
    fn model_round_trip_keeps_nested_state_and_unicode() {
        let model = MenuBarModel::new(vec![MenuItemModel::new("view", "表示").children(vec![
                MenuEntry::Separator,
                MenuEntry::Item(
                    MenuItemModel::new("minimap", "Minimap")
                        .checked(true)
                        .enabled(false)
                        .shortcut("Cmd+M"),
                ),
            ])]);
        let parsed = parse_menu(&model.to_json()).unwrap();
        assert_eq!(parsed.items[0].title, "表示");
        assert!(parsed.items[0].items[0].separator);
        assert!(parsed.items[0].items[1].checked);
        assert!(!parsed.items[0].items[1].enabled);
        assert!(parse_menu("").unwrap().items.is_empty());
        assert!(parse_menu("{bad}").is_err());
    }

    #[test]
    fn shortcuts_preserve_explicit_control_and_do_not_capture_chord_prefixes() {
        let cmd = parse_shortcut("Cmd+Shift+S").unwrap();
        assert_eq!(cmd.key, "s");
        assert_eq!(
            cmd.modifiers,
            NSEventModifierFlags::NSEventModifierFlagCommand
                | NSEventModifierFlags::NSEventModifierFlagShift
        );
        assert_eq!(
            parse_shortcut("Ctrl+`").unwrap().modifiers,
            NSEventModifierFlags::NSEventModifierFlagControl
        );
        assert_eq!(parse_shortcut("F12").unwrap().key, "\u{f70f}");
        assert_eq!(
            parse_shortcut("Primary+O").unwrap().modifiers,
            NSEventModifierFlags::NSEventModifierFlagCommand
        );
        for chord in [
            "Cmd+K Cmd+T",
            "Ctrl+K, Ctrl+M",
            "Cmd+K → Cmd+I",
            "unknown",
            "F36",
        ] {
            assert!(parse_shortcut(chord).is_none(), "{chord}");
        }
    }

    #[test]
    fn editor_accelerators_and_quit_stay_in_focused_winit_dispatch() {
        let model = MenuBarModel::new(vec![
            MenuItemModel::new("edit", "Edit").children(
                ["A", "C", "X", "V", "Z", "Q"]
                    .into_iter()
                    .map(|key| {
                        MenuEntry::Item(MenuItemModel::new(key, key).shortcut(format!("Cmd+{key}")))
                    })
                    .collect(),
            ),
        ]);
        let spec = parse_menu(&model.to_json()).unwrap();
        let mut shortcuts = Vec::new();
        collect_shortcuts(&spec.items, &mut shortcuts);
        let command = NSEventModifierFlags::NSEventModifierFlagCommand;
        for key in ["a", "c", "x", "v", "z", "q", "A"] {
            assert!(is_model_shortcut(&shortcuts, key, command));
        }
        assert!(is_model_shortcut(
            &shortcuts,
            "a",
            command | NSEventModifierFlags::NSEventModifierFlagCapsLock
        ));
        assert!(!is_model_shortcut(
            &shortcuts,
            "a",
            NSEventModifierFlags::empty()
        ));
        // Hide remains owned by the standard app menu, while model Quit is
        // routed through the application's existing unsaved-close handler.
        assert!(!is_model_shortcut(&shortcuts, "h", command));
        assert_eq!(find_quit(&spec.items, true).unwrap().0.id, "Q");
    }

    #[test]
    fn quit_in_disabled_parent_preserves_disabled_state() {
        let menu = MenuBarModel::new(vec![MenuItemModel::app().enabled(false).children(vec![
            MenuEntry::Item(MenuItemModel::new("quit", "Quit").shortcut("Cmd+Q")),
        ])]);
        let spec = parse_menu(&menu.to_json()).unwrap();
        assert!(!find_quit(&spec.items, true).unwrap().1);
    }
}
