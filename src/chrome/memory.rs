use std::{cell::RefCell, collections::VecDeque, rc::Rc};

use gtk::prelude::*;

use super::{
    strip::set_accessible_name,
    theme::{Shade, Token},
    ChromeIntent,
};
use crate::browser::{ThemeMode, VisitId};

const ERROR_LIMIT: usize = 5;

pub(super) enum MemoryEvent {
    Run(ChromeIntent),
    Menu(bool),
}

/// The Memory menu, opened from the dot at the end of the current-page pill.
pub(super) struct Memory {
    pub button: gtk::MenuButton,
    dot: gtk::DrawingArea,
    popover: gtk::Popover,
    toggle: gtk::Switch,
    read: gtk::Button,
    theme: gtk::ComboBoxText,
    errors: gtk::Box,
    state: RefCell<State>,
    on_event: Box<dyn Fn(MemoryEvent)>,
}

#[derive(Default)]
struct State {
    capture_enabled: bool,
    current: Option<VisitId>,
    errors: VecDeque<String>,
    shade: Option<Shade>,
}

impl Memory {
    pub fn new(on_event: impl Fn(MemoryEvent) + 'static) -> Rc<Self> {
        let button = gtk::MenuButton::new();
        button.set_widget_name("memory-menu");
        button.style_context().add_class("memory-dot");
        button.set_focus_on_click(false);
        button.set_size_request(18, 18);
        set_accessible_name(&button, "Memory");
        let dot = gtk::DrawingArea::new();
        dot.set_size_request(18, 18);
        button.add(&dot);
        let popover = gtk::Popover::new(Some(&button));
        popover.set_widget_name("memory-popover");
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_size_request(260, -1);
        let heading = gtk::Label::new(Some("Memory"));
        heading.set_xalign(0.);
        heading.style_context().add_class("menu-heading");
        column.pack_start(&heading, false, false, 0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        row.style_context().add_class("menu-row");
        row.set_tooltip_text(Some(
            "Keep the text of opened pages on this computer. Form fields are never kept.",
        ));
        let label = gtk::Label::new(Some("Remember pages"));
        label.set_xalign(0.);
        row.pack_start(&label, true, true, 0);
        let toggle = gtk::Switch::new();
        toggle.set_widget_name("automatic-capture");
        toggle.set_valign(gtk::Align::Center);
        set_accessible_name(&toggle, "Remember pages");
        row.pack_start(&toggle, false, false, 0);
        column.pack_start(&row, false, false, 0);
        let remember = menu_row("remember-selection", "Remember selection", Some("Ctrl+S"));
        column.pack_start(&remember, false, false, 0);
        let read = menu_row("read-captures", "Read saved text", None);
        column.pack_start(&read, false, false, 0);
        let theme = gtk::ComboBoxText::new();
        theme.set_widget_name("theme-mode");
        for (id, text) in [
            ("system", "Theme: System"),
            ("light", "Theme: Light"),
            ("dark", "Theme: Dark"),
        ] {
            theme.append(Some(id), text);
        }
        column.pack_start(&theme, false, false, 6);
        let errors = gtk::Box::new(gtk::Orientation::Vertical, 0);
        errors.set_widget_name("memory-errors");
        errors.set_no_show_all(true);
        column.pack_start(&errors, false, false, 0);
        column.show_all();
        popover.add(&column);
        button.set_popover(Some(&popover));
        let memory = Rc::new(Self {
            button,
            dot,
            popover,
            toggle,
            read,
            theme,
            errors,
            state: RefCell::new(State::default()),
            on_event: Box::new(on_event),
        });
        let weak = Rc::downgrade(&memory);
        memory.theme.connect_changed(move |combo| {
            if let Some(memory) = weak.upgrade() {
                let mode = match combo.active_id().as_deref() {
                    Some("light") => ThemeMode::Light,
                    Some("dark") => ThemeMode::Dark,
                    _ => ThemeMode::System,
                };
                (memory.on_event)(MemoryEvent::Run(ChromeIntent::SetTheme(mode)));
            }
        });
        let weak = Rc::downgrade(&memory);
        memory.toggle.connect_active_notify(move |toggle| {
            let Some(memory) = weak.upgrade() else { return };
            let active = toggle.is_active();
            if memory.state.borrow().capture_enabled != active {
                memory.state.borrow_mut().capture_enabled = active;
                memory.dot.queue_draw();
                (memory.on_event)(MemoryEvent::Run(ChromeIntent::SetCaptureEnabled(active)));
            }
        });
        let weak = Rc::downgrade(&memory);
        remember.connect_clicked(move |_| {
            if let Some(memory) = weak.upgrade() {
                memory.popover.popdown();
                (memory.on_event)(MemoryEvent::Run(ChromeIntent::RememberSelection));
            }
        });
        let weak = Rc::downgrade(&memory);
        memory.read.connect_clicked(move |_| {
            let Some(memory) = weak.upgrade() else { return };
            let current = memory.state.borrow().current;
            memory.popover.popdown();
            if let Some(visit) = current {
                (memory.on_event)(MemoryEvent::Run(ChromeIntent::ReadCaptures(visit)));
            }
        });
        let weak = Rc::downgrade(&memory);
        memory.popover.connect_show(move |_| {
            if let Some(memory) = weak.upgrade() {
                (memory.on_event)(MemoryEvent::Menu(true));
            }
        });
        let weak = Rc::downgrade(&memory);
        memory.popover.connect_closed(move |_| {
            if let Some(memory) = weak.upgrade() {
                (memory.on_event)(MemoryEvent::Menu(false));
            }
        });
        let weak = Rc::downgrade(&memory);
        memory.dot.connect_draw(move |area, cr| {
            if let Some(memory) = weak.upgrade() {
                let state = memory.state.borrow();
                let shade = state.shade.unwrap_or(Shade::Light);
                let (w, h) = (
                    f64::from(area.allocated_width()),
                    f64::from(area.allocated_height()),
                );
                cr.arc(w / 2., h / 2., 3.5, 0., 2. * std::f64::consts::PI);
                if state.capture_enabled {
                    shade.source(cr, Token::Green);
                    let _ = cr.fill();
                } else {
                    shade.source(cr, Token::Faint);
                    cr.set_line_width(1.5);
                    let _ = cr.stroke();
                }
            }
            glib::Propagation::Stop
        });
        memory
    }

    pub fn present(
        &self,
        capture_enabled: bool,
        current: Option<VisitId>,
        shade: Shade,
        theme: ThemeMode,
    ) {
        let mut state = self.state.borrow_mut();
        state.capture_enabled = capture_enabled;
        state.current = current;
        state.shade = Some(shade);
        drop(state);
        self.theme.set_active_id(Some(match theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        }));
        self.toggle.set_active(capture_enabled);
        self.read.set_sensitive(current.is_some());
        self.button.set_tooltip_text(Some(if capture_enabled {
            "Memory · Remembering pages"
        } else {
            "Memory · Not remembering pages"
        }));
        self.dot.queue_draw();
    }

    pub fn record_error(&self, message: &str) {
        let mut state = self.state.borrow_mut();
        state.errors.push_front(message.to_owned());
        state.errors.truncate(ERROR_LIMIT);
        let errors: Vec<_> = state.errors.iter().cloned().collect();
        drop(state);
        for child in self.errors.children() {
            self.errors.remove(&child);
        }
        let separator = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        separator.style_context().add_class("menu-separator");
        self.errors.pack_start(&separator, false, false, 0);
        let heading = gtk::Label::new(Some("Recent errors"));
        heading.set_xalign(0.);
        heading.style_context().add_class("menu-heading");
        self.errors.pack_start(&heading, false, false, 0);
        for error in errors {
            let label = gtk::Label::new(Some(&error));
            label.set_xalign(0.);
            label.set_line_wrap(true);
            label.set_max_width_chars(40);
            label.style_context().add_class("menu-error");
            self.errors.pack_start(&label, false, false, 0);
        }
        self.errors.show_all();
    }
}

fn menu_row(name: &str, text: &str, shortcut: Option<&str>) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_widget_name(name);
    button.style_context().add_class("menu-row");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.);
    content.pack_start(&label, true, true, 0);
    if let Some(shortcut) = shortcut {
        let kbd = gtk::Label::new(Some(shortcut));
        kbd.style_context().add_class("kbd");
        content.pack_start(&kbd, false, false, 0);
    }
    button.add(&content);
    button
}
