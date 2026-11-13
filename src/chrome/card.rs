use std::{cell::Cell, rc::Rc};

use gtk::{gdk, pango, prelude::*};

use super::{favicon::Favicons, strip::site_line, tabs::plural};
use crate::{
    browser::{Visit, VisitId, VisitPreview, VisitState},
    forest::ForestProjection,
};

pub(super) const WIDTH: i32 = 304;

type Action = fn(&Card) -> Option<CardEvent>;

pub(super) enum CardEvent {
    Enter,
    Leave,
    Open(VisitId),
    Branch(VisitId),
    BackToSource(VisitId),
    OpenComment(VisitId),
}

/// The hover card under the open strip.
pub(super) struct Card {
    /// Draws the card's shadow around `root`, which alone takes pointer input.
    pub frame: gtk::Box,
    pub root: gtk::EventBox,
    icon: gtk::Image,
    title: gtk::Label,
    meta: gtk::Label,
    tags: gtk::Box,
    snippet: gtk::Label,
    source: gtk::Box,
    source_label: gtk::Label,
    back: gtk::Button,
    comment: gtk::Button,
    open: gtk::Button,
    branch: gtk::Button,
    favicons: Rc<Favicons>,
    visit: Cell<Option<VisitId>>,
    parent: Cell<Option<VisitId>>,
    on_event: Box<dyn Fn(CardEvent)>,
}

impl Card {
    pub fn new(favicons: Rc<Favicons>, on_event: impl Fn(CardEvent) + 'static) -> Rc<Self> {
        let root = gtk::EventBox::new();
        root.set_widget_name("card");
        root.set_no_show_all(true);
        root.add_events(gdk::EventMask::ENTER_NOTIFY_MASK | gdk::EventMask::LEAVE_NOTIFY_MASK);
        let body = gtk::Box::new(gtk::Orientation::Vertical, 0);
        body.style_context().add_class("card");
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 9);
        let icon = gtk::Image::new();
        icon.set_valign(gtk::Align::Start);
        icon.set_margin_top(2);
        head.pack_start(&icon, false, false, 0);
        let title = label("card-title", 2);
        head.pack_start(&title, true, true, 0);
        body.pack_start(&head, false, false, 0);
        let indented = gtk::Box::new(gtk::Orientation::Vertical, 0);
        indented.set_margin_start(25);
        let meta = label("card-meta", 1);
        indented.pack_start(&meta, false, false, 0);
        let tags = gtk::Box::new(gtk::Orientation::Horizontal, 5);
        tags.set_margin_top(7);
        indented.pack_start(&tags, false, false, 0);
        let snippet = label("card-snippet", 4);
        snippet.set_margin_top(8);
        indented.pack_start(&snippet, false, false, 0);
        let source = gtk::Box::new(gtk::Orientation::Vertical, 6);
        source.set_margin_top(8);
        let source_label = label("card-meta", 2);
        source.pack_start(&source_label, false, false, 0);
        let source_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let back = action("card-source", "Back to source");
        let comment = action("card-comment", "Open comment");
        source_actions.pack_start(&back, false, false, 0);
        source_actions.pack_start(&comment, false, false, 0);
        source.pack_start(&source_actions, false, false, 0);
        indented.pack_start(&source, false, false, 0);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        actions.set_margin_top(10);
        let open = action("card-open", "Open");
        let branch = action("card-branch", "New branch here");
        actions.pack_start(&open, false, false, 0);
        actions.pack_start(&branch, false, false, 0);
        indented.pack_start(&actions, false, false, 0);
        body.pack_start(&indented, false, false, 0);
        root.add(&body);
        body.show_all();
        let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
        frame.style_context().add_class("card-frame");
        frame.pack_start(&root, true, true, 0);
        frame.show();
        let card = Rc::new(Self {
            frame,
            root,
            icon,
            title,
            meta,
            tags,
            snippet,
            source,
            source_label,
            back,
            comment,
            open,
            branch,
            favicons,
            visit: Cell::new(None),
            parent: Cell::new(None),
            on_event: Box::new(on_event),
        });
        card.connect();
        card
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.root.connect_enter_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                if let Some(card) = weak.upgrade() {
                    (card.on_event)(CardEvent::Enter);
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.root.connect_leave_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                if let Some(card) = weak.upgrade() {
                    (card.on_event)(CardEvent::Leave);
                }
            }
            glib::Propagation::Proceed
        });
        let buttons: [(&gtk::Button, Action); 4] = [
            (&self.open, |card| card.visit.get().map(CardEvent::Open)),
            (&self.branch, |card| card.visit.get().map(CardEvent::Branch)),
            (&self.back, |card| {
                card.parent.get().map(CardEvent::BackToSource)
            }),
            (&self.comment, |card| {
                card.visit.get().map(CardEvent::OpenComment)
            }),
        ];
        for (button, event) in buttons {
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(card) = weak.upgrade() {
                    if let Some(event) = event(&card) {
                        (card.on_event)(event);
                    }
                }
            });
        }
    }

    pub fn visit(&self) -> Option<VisitId> {
        self.visit.get().filter(|_| self.root.is_visible())
    }

    pub fn show(self: &Rc<Self>, forest: &ForestProjection, id: VisitId, preview: VisitPreview) {
        let Some(visit) = forest.visit(id) else {
            return;
        };
        self.visit.set(Some(id));
        let surface = self.favicons.surface(visit, 16, self.root.scale_factor());
        self.icon.set_from_surface(Some(&surface));
        self.title.set_text(&preview.title);
        let after = forest.descendant_count(id).unwrap_or(0);
        let mut meta = site_line(visit);
        if after > 0 {
            meta.push_str(&format!(" · {} after", plural(after, "page")));
        }
        self.meta.set_text(&meta);
        for tag in self.tags.children() {
            self.tags.remove(&tag);
        }
        let current = forest.current() == Some(id);
        for (on, text) in [
            (current, "Current page"),
            (visit.state == VisitState::Queued, "Pending"),
        ] {
            if on {
                let tag = gtk::Label::new(Some(text));
                tag.style_context().add_class("card-tag");
                self.tags.pack_start(&tag, false, false, 0);
            }
        }
        self.tags.show_all();
        self.tags.set_visible(!self.tags.children().is_empty());
        let snippet = preview
            .snippet
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        self.snippet.set_text(&snippet);
        self.snippet.set_visible(!snippet.is_empty());
        self.open.set_label(if current { "Reload" } else { "Open" });
        self.show_source(visit, current);
        self.root.show();
        if let Some(outer) = self.frame.parent() {
            outer.show();
        }
    }

    /// On the current page, the card carries the page's source context and its actions.
    fn show_source(&self, visit: &Visit, current: bool) {
        self.parent.set(visit.parent);
        let source = visit.source.as_ref().filter(|_| current);
        self.source.set_visible(source.is_some());
        let Some(source) = source else { return };
        let host = source.page_url.host_str().unwrap_or("the source");
        let text = if source.link_text.is_empty() {
            format!("From {host}")
        } else {
            format!("From {host} · {}", source.link_text)
        };
        self.source_label.set_text(&text);
        self.back.set_sensitive(visit.parent.is_some());
        self.comment.set_sensitive(source.citation_url.is_some());
    }

    pub fn hide(&self) {
        self.root.hide();
        if let Some(outer) = self.frame.parent() {
            outer.hide();
        }
    }
}

fn label(class: &str, lines: i32) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.set_xalign(0.);
    label.set_line_wrap(true);
    label.set_line_wrap_mode(pango::WrapMode::WordChar);
    label.set_lines(lines);
    label.set_ellipsize(pango::EllipsizeMode::End);
    label.set_max_width_chars(36);
    label.style_context().add_class(class);
    label
}

fn action(name: &str, text: &str) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.set_widget_name(name);
    button.style_context().add_class("card-action");
    button.set_focus_on_click(false);
    button
}
