use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

use gtk::{pango, prelude::*};

use super::{
    favicon::Favicons,
    icons::{icon, Icon},
    strip::set_accessible_name,
    theme::FONT,
};
use crate::{browser::TreeId, forest::ForestProjection};

const TAB_GAP: i32 = 2;
const ARROW: i32 = 32;

pub(super) enum TabEvent {
    Select(TreeId),
    NewTree,
    Search,
    Forest,
}

pub(super) struct Tabs {
    pub root: gtk::Box,
    row: gtk::Box,
    right: gtk::Box,
    new_tree: gtk::Button,
    overflow: gtk::MenuButton,
    overflow_rows: gtk::Box,
    favicons: Rc<Favicons>,
    tabs: RefCell<Vec<Tab>>,
    width: Cell<i32>,
    on_event: Box<dyn Fn(TabEvent)>,
}

struct Tab {
    id: TreeId,
    button: gtk::Button,
    image: gtk::Image,
    title: gtk::Label,
    count: gtk::Label,
    line: gtk::Box,
    width: i32,
    active: bool,
}

impl Tabs {
    pub fn new(favicons: Rc<Favicons>, on_event: impl Fn(TabEvent) + 'static) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        root.set_widget_name("tabs");
        let left = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let row = gtk::Box::new(gtk::Orientation::Horizontal, TAB_GAP);
        row.set_valign(gtk::Align::End);
        left.pack_start(&row, false, false, 0);
        let overflow = gtk::MenuButton::new();
        overflow.set_widget_name("tab-overflow");
        overflow.style_context().add_class("icon-button");
        overflow.add(&icon(Icon::Chevron, 14));
        overflow.set_tooltip_text(Some("More trees"));
        overflow.set_valign(gtk::Align::Center);
        overflow.set_focus_on_click(false);
        overflow.set_no_show_all(true);
        let popover = gtk::Popover::new(Some(&overflow));
        popover.set_widget_name("tab-overflow-list");
        let overflow_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        overflow_rows.set_size_request(280, -1);
        popover.add(&overflow_rows);
        overflow.set_popover(Some(&popover));
        left.pack_start(&overflow, false, false, 2);
        let new_tree = icon_button(Icon::Plus, "new-tree", "New tree · Ctrl+T");
        left.pack_start(&new_tree, false, false, 2);
        root.pack_start(&left, true, true, 0);

        let right = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        right.set_valign(gtk::Align::Center);
        let search = gtk::Button::new();
        search.set_widget_name("search-pill");
        search.style_context().add_class("search-pill");
        search.set_focus_on_click(false);
        search.set_tooltip_text(Some("Search your forest or go to an address · Ctrl+K"));
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.pack_start(&icon(Icon::Search, 14), false, false, 0);
        let label = gtk::Label::new(Some("Search or go…"));
        label.set_xalign(0.);
        content.pack_start(&label, true, true, 0);
        let kbd = gtk::Label::new(Some("Ctrl K"));
        kbd.style_context().add_class("kbd");
        content.pack_start(&kbd, false, false, 0);
        search.add(&content);
        right.pack_start(&search, false, false, 0);
        let agent = icon_button(Icon::Spark, "agent", "Agent · Coming soon");
        agent.set_sensitive(false);
        right.pack_start(&agent, false, false, 0);
        let forest = icon_button(Icon::Forest, "forest", "Forest view · Ctrl+E");

        right.pack_start(&forest, false, false, 0);
        root.pack_end(&right, false, false, 0);

        let tabs = Rc::new(Self {
            root,
            row,
            right,
            new_tree,
            overflow,
            overflow_rows,
            favicons,
            tabs: RefCell::new(Vec::new()),
            width: Cell::new(0),
            on_event: Box::new(on_event),
        });
        let weak = Rc::downgrade(&tabs);
        tabs.new_tree.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                (tabs.on_event)(TabEvent::NewTree);
            }
        });
        let weak = Rc::downgrade(&tabs);
        forest.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                (tabs.on_event)(TabEvent::Forest);
            }
        });
        let weak = Rc::downgrade(&tabs);
        search.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                (tabs.on_event)(TabEvent::Search);
            }
        });
        let weak = Rc::downgrade(&tabs);
        tabs.root.connect_size_allocate(move |_, allocation| {
            let Some(tabs) = weak.upgrade() else { return };
            if tabs.width.replace(allocation.width()) != allocation.width() {
                let weak = Rc::downgrade(&tabs);
                glib::idle_add_local_once(move || {
                    if let Some(tabs) = weak.upgrade() {
                        tabs.fit();
                    }
                });
            }
        });
        tabs
    }

    pub fn present(self: &Rc<Self>, forest: &ForestProjection) {
        let active = forest.current().and_then(|id| forest.tree_of(id));
        let mut reuse: HashMap<TreeId, Tab> = self
            .tabs
            .borrow_mut()
            .drain(..)
            .map(|tab| (tab.id, tab))
            .collect();
        let scale = self.root.scale_factor();
        let mut tabs = Vec::with_capacity(forest.trees().len());
        for (index, tree) in forest.trees().iter().enumerate() {
            let mut tab = reuse.remove(&tree.id).unwrap_or_else(|| self.tab(tree.id));
            let root = forest.visit(tree.id.root()).unwrap();
            tab.image
                .set_from_surface(Some(&self.favicons.surface(root, 14, scale)));
            tab.title.set_text(&tree.title);
            tab.count.set_text(&tree.pending_count.to_string());
            tab.count.set_visible(tree.pending_count > 0);
            tab.active = Some(tree.id) == active;
            tab.line.set_visible(tab.active);
            let context = tab.button.style_context();
            if tab.active {
                context.add_class("on");
            } else {
                context.remove_class("on");
            }
            let pages = plural(tree.page_count, "page");
            let pending = if tree.pending_count > 0 {
                format!(", {} pending", tree.pending_count)
            } else {
                String::new()
            };
            let shortcut = if index < 9 {
                format!(" · Ctrl+{}", index + 1)
            } else {
                String::new()
            };
            tab.button.set_tooltip_text(Some(&format!(
                "{} · {pages}{pending}{shortcut}",
                tree.title
            )));
            set_accessible_name(&tab.button, &format!("{}, {pages}{pending}", tree.title));
            tab.width = self.tab_width(&tree.title, tree.pending_count);
            tab.button.set_size_request(tab.width, 32);
            self.row.reorder_child(&tab.button, index as i32);
            tabs.push(tab);
        }
        for tab in reuse.into_values() {
            self.row.remove(&tab.button);
        }
        *self.tabs.borrow_mut() = tabs;
        self.fit();
    }

    fn tab(self: &Rc<Self>, id: TreeId) -> Tab {
        let button = gtk::Button::new();
        button.set_widget_name(&format!("tab-{}", id.root()));
        button.style_context().add_class("tab");
        button.set_focus_on_click(false);
        button.set_valign(gtk::Align::End);
        let overlay = gtk::Overlay::new();
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        let image = gtk::Image::new();
        content.pack_start(&image, false, false, 0);
        let title = gtk::Label::new(None);
        title.set_xalign(0.);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.set_max_width_chars(1);
        title.style_context().add_class("tab-title");
        content.pack_start(&title, true, true, 0);
        let count = gtk::Label::new(None);
        count.style_context().add_class("count");
        count.set_valign(gtk::Align::Center);
        count.set_no_show_all(true);
        content.pack_start(&count, false, false, 0);
        overlay.add(&content);
        let line = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        line.style_context().add_class("tab-line");
        line.set_valign(gtk::Align::Start);
        line.set_size_request(-1, 2);
        line.set_no_show_all(true);
        overlay.add_overlay(&line);
        button.add(&overlay);
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(tabs) = weak.upgrade() {
                (tabs.on_event)(TabEvent::Select(id));
            }
        });
        self.row.pack_start(&button, false, false, 0);
        button.show_all();
        Tab {
            id,
            button,
            image,
            title,
            count,
            line,
            width: 0,
            active: false,
        }
    }

    fn tab_width(&self, title: &str, pending: usize) -> i32 {
        let measure = |text: &str, size: f64| {
            let layout = self.row.create_pango_layout(Some(text));
            let mut font = pango::FontDescription::from_string(FONT);
            font.set_absolute_size(size * f64::from(pango::SCALE));
            font.set_weight(pango::Weight::Semibold);
            layout.set_font_description(Some(&font));
            layout.pixel_size().0
        };
        let count = if pending > 0 {
            7 + 10 + measure(&pending.to_string(), 10.5)
        } else {
            0
        };
        (10 + 14 + 7 + measure(title, 12.5) + count + 11).min(200)
    }

    /// Hides the trees that do not fit behind the overflow arrow, keeping creation order
    /// and the active tree visible.
    fn fit(self: &Rc<Self>) {
        let tabs = self.tabs.borrow();
        let width = self.root.allocated_width();
        if width <= 1 {
            return;
        }
        let fixed = self.right.preferred_width().1 + self.new_tree.preferred_width().1 + 4 + 14;
        let total: i32 = tabs.iter().map(|tab| tab.width + TAB_GAP).sum();
        let mut visible: Vec<bool> = vec![true; tabs.len()];
        if total > width - fixed {
            let room = width - fixed - ARROW - 4;
            let mut used = 0;
            for (i, tab) in tabs.iter().enumerate() {
                used += tab.width + TAB_GAP;
                visible[i] = used <= room;
            }
            if let Some(active) = tabs.iter().position(|tab| tab.active) {
                if !visible[active] {
                    let needed = tabs[active].width + TAB_GAP;
                    let mut used: i32 = tabs
                        .iter()
                        .zip(&visible)
                        .filter(|(_, v)| **v)
                        .map(|(tab, _)| tab.width + TAB_GAP)
                        .sum();
                    for i in (0..tabs.len()).rev() {
                        if used + needed <= room {
                            break;
                        }
                        if visible[i] {
                            visible[i] = false;
                            used -= tabs[i].width + TAB_GAP;
                        }
                    }
                    visible[active] = true;
                }
            }
        }
        for (tab, visible) in tabs.iter().zip(&visible) {
            tab.button.set_visible(*visible);
        }
        for row in self.overflow_rows.children() {
            self.overflow_rows.remove(&row);
        }
        let heading = gtk::Label::new(Some("More trees"));
        heading.set_xalign(0.);
        heading.style_context().add_class("menu-heading");
        self.overflow_rows.pack_start(&heading, false, false, 0);
        let mut hidden = 0;
        for (tab, _) in tabs.iter().zip(&visible).filter(|(_, v)| !**v) {
            hidden += 1;
            let row = gtk::Button::new();
            row.set_widget_name("overflow-tree");
            row.style_context().add_class("menu-row");
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 9);
            if let Some(surface) = tab.image.surface() {
                content.pack_start(&gtk::Image::from_surface(Some(&surface)), false, false, 0);
            }
            let title = gtk::Label::new(Some(&tab.title.text()));
            title.set_xalign(0.);
            title.set_ellipsize(pango::EllipsizeMode::End);
            content.pack_start(&title, true, true, 0);
            if tab.count.is_visible() {
                let count = gtk::Label::new(Some(&tab.count.text()));
                count.style_context().add_class("count");
                content.pack_start(&count, false, false, 0);
            }
            row.add(&content);
            let id = tab.id;
            let weak = Rc::downgrade(self);
            row.connect_clicked(move |row| {
                if let Some(popover) = row
                    .ancestor(gtk::Popover::static_type())
                    .and_then(|p| p.downcast::<gtk::Popover>().ok())
                {
                    popover.popdown();
                }
                if let Some(tabs) = weak.upgrade() {
                    (tabs.on_event)(TabEvent::Select(id));
                }
            });
            self.overflow_rows.pack_start(&row, false, false, 0);
        }
        self.overflow_rows.show_all();
        self.overflow.set_visible(hidden > 0);
    }
}

fn icon_button(glyph: Icon, name: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::new();
    button.set_widget_name(name);
    button.style_context().add_class("icon-button");
    button.add(&icon(glyph, 16));
    button.set_tooltip_text(Some(tooltip));
    button.set_valign(gtk::Align::Center);
    button.set_focus_on_click(false);
    set_accessible_name(&button, tooltip.split(" · ").next().unwrap_or(tooltip));
    button
}

pub(super) fn plural(count: usize, word: &str) -> String {
    format!("{count} {word}{}", if count == 1 { "" } else { "s" })
}
