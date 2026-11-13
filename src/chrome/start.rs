use super::{
    favicon::Favicons,
    forest_view::dapple,
    icons::{icon, Icon},
    theme::Shade,
    ChromeIntent,
};
use crate::browser::StartSuggestions;
use gtk::prelude::*;
use std::{cell::Cell, rc::Rc};

pub(super) struct StartPage {
    pub root: gtk::EventBox,
    content: gtk::Box,
    favicons: Rc<Favicons>,
    shade: Cell<Shade>,
    on_event: Rc<dyn Fn(Option<ChromeIntent>)>,
}

impl StartPage {
    pub fn new(
        favicons: Rc<Favicons>,
        on_event: impl Fn(Option<ChromeIntent>) + 'static,
    ) -> Rc<Self> {
        let root = gtk::EventBox::new();
        root.set_widget_name("start-page");
        root.set_no_show_all(true);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 20);
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        content.set_size_request(540, -1);
        let overlay = gtk::Overlay::new();
        let background = gtk::DrawingArea::new();
        background.set_hexpand(true);
        background.set_vexpand(true);
        overlay.add(&background);
        overlay.add_overlay(&content);
        root.add(&overlay);
        overlay.show_all();
        let page = Rc::new(Self {
            root,
            content,
            favicons,
            shade: Cell::new(Shade::Light),
            on_event: Rc::new(on_event),
        });
        let w = Rc::downgrade(&page);
        background.connect_draw(move |area, cr| {
            if let Some(p) = w.upgrade() {
                dapple(
                    cr,
                    p.shade.get(),
                    area.allocated_width(),
                    area.allocated_height(),
                );
            }
            glib::Propagation::Proceed
        });
        page
    }
    pub fn set_shade(&self, shade: Shade) {
        self.shade.set(shade);
        self.root.queue_draw();
    }
    pub fn present(&self, suggestions: &StartSuggestions) {
        for child in self.content.children() {
            self.content.remove(&child);
        }
        let title = gtk::Label::new(Some("New tree"));
        title.style_context().add_class("start-title");
        self.content.pack_start(&title, false, false, 0);
        let search = gtk::Button::new();
        search.set_widget_name("start-search");
        search.set_size_request(540, 46);
        search.set_halign(gtk::Align::Center);
        search.style_context().add_class("start-search");
        let field = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        field.pack_start(&icon(Icon::Search, 16), false, false, 0);
        let hint = gtk::Label::new(Some("An address, a search, or a page in your forest"));
        hint.set_xalign(0.);
        hint.set_ellipsize(gtk::pango::EllipsizeMode::End);
        field.pack_start(&hint, true, true, 0);
        let kbd = gtk::Label::new(Some("Ctrl K"));
        kbd.set_valign(gtk::Align::Center);
        kbd.style_context().add_class("kbd");
        field.pack_start(&kbd, false, false, 0);
        search.add(&field);
        let on_event = self.on_event.clone();
        search.connect_clicked(move |_| on_event(None));
        self.content.pack_start(&search, false, false, 0);
        let tiles = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        tiles.set_halign(gtk::Align::Center);
        for (index, shortcut) in suggestions.shortcuts.iter().enumerate() {
            let host = shortcut.url.host_str().unwrap_or_default();
            let button = gtk::Button::new();
            button.set_widget_name(&format!("start-shortcut-{index}"));
            button.style_context().add_class("shortcut-tile");
            button.set_tooltip_text(Some(shortcut.url.as_str()));
            let column = gtk::Box::new(gtk::Orientation::Vertical, 8);
            let glyph = gtk::Image::from_surface(Some(&self.favicons.url_surface(
                &shortcut.url,
                true,
                24,
                self.root.scale_factor(),
            )));
            glyph.style_context().add_class("shortcut-icon");
            column.pack_start(&glyph, false, false, 0);
            let label = gtk::Label::new(Some(if shortcut.title.is_empty() {
                host
            } else {
                &shortcut.title
            }));
            label.set_max_width_chars(10);
            label.set_ellipsize(gtk::pango::EllipsizeMode::End);
            column.pack_start(&label, false, false, 0);
            button.add(&column);
            let on_event = self.on_event.clone();
            let url = shortcut.url.clone();
            button.connect_clicked(move |_| {
                on_event(Some(ChromeIntent::Navigate {
                    url: url.clone(),
                    target: crate::browser::NavigationTarget::Root,
                }))
            });
            tiles.pack_start(&button, false, false, 0);
        }
        self.content.pack_start(&tiles, false, false, 0);
        if suggestions.shortcuts.is_empty() {
            let empty = gtk::Label::new(Some(
                "Your frequent pages will appear here as your forest grows.",
            ));
            empty.style_context().add_class("menu-detail");
            self.content.pack_start(&empty, false, false, 0);
        }
        let paths = gtk::Box::new(gtk::Orientation::Vertical, 2);
        paths.set_size_request(540, -1);
        paths.set_halign(gtk::Align::Center);
        let label = gtk::Label::new(Some("Usual paths"));
        label.set_xalign(0.);
        label.style_context().add_class("menu-heading");
        paths.pack_start(&label, false, false, 0);
        for (index, path) in suggestions.paths.iter().enumerate() {
            let scale = self.root.scale_factor();
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
            let chain = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            for (step, url) in [&path.first, &path.second].into_iter().enumerate() {
                if step > 0 {
                    let link = gtk::Label::new(Some("→"));
                    link.style_context().add_class("path-link");
                    chain.pack_start(&link, false, false, 0);
                }
                let favicon = gtk::Image::from_surface(Some(
                    &self.favicons.url_surface(url, true, 14, scale),
                ));
                favicon.style_context().add_class("path-icon");
                chain.pack_start(&favicon, false, false, 0);
            }
            row.pack_start(&chain, false, false, 0);
            let text = gtk::Label::new(Some(&format!(
                "{}, then {}",
                short_place(&path.first),
                short_place(&path.second)
            )));
            text.set_xalign(0.);
            text.set_ellipsize(gtk::pango::EllipsizeMode::End);
            text.style_context().add_class("path-text");
            row.pack_start(&text, true, true, 0);
            let count = gtk::Label::new(Some(&format!("{} times", path.visits)));
            count.style_context().add_class("menu-detail");
            row.pack_start(&count, false, false, 0);
            let button = gtk::Button::new();
            button.add(&row);
            button.set_widget_name(&format!("start-path-{index}"));
            button.set_tooltip_text(Some(&format!("{}\n{}", path.first, path.second)));
            button.style_context().add_class("menu-row");
            let on_event = self.on_event.clone();
            let path = path.clone();
            button.connect_clicked(move |_| on_event(Some(ChromeIntent::OpenPath(path.clone()))));
            paths.pack_start(&button, false, false, 0);
        }
        if suggestions.paths.is_empty() {
            let empty = gtk::Label::new(Some("Follow links to build paths you can revisit."));
            empty.set_xalign(0.);
            empty.style_context().add_class("menu-detail");
            paths.pack_start(&empty, false, false, 0);
        }
        self.content.pack_start(&paths, false, false, 0);
        self.content.show_all();
    }
}

fn short_place(url: &url::Url) -> &str {
    url.host_str()
        .unwrap_or_default()
        .trim_start_matches("www.")
}
