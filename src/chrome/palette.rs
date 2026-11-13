use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, pango, prelude::*};
use url::Url;

use super::{
    favicon::Favicons,
    icons::{icon, Icon},
    ChromeIntent, PaletteScope, PaletteSeed,
};
use crate::{
    address::{destination, web_search, Destination},
    browser::{NavigationTarget, SearchHit, VisitId, VisitState},
    forest::ForestProjection,
};

const GROUP_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PaletteItem {
    Page(VisitId),
    Saved {
        capture: i64,
        title: String,
        excerpt: String,
    },
    Go(Url),
    WebSearch {
        url: Url,
        query: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Group {
    Pending,
    Forest,
    Saved,
    Elsewhere,
}

impl Group {
    fn title(self) -> &'static str {
        match self {
            Group::Pending => "Pending in this tree",
            Group::Forest => "In your forest",
            Group::Saved => "Saved text",
            Group::Elsewhere => "Elsewhere",
        }
    }
}

/// The palette's results for `query`, in display order, and the item Enter runs first.
/// Pages and saved text stay distinct: a saved-text hit opens the local reader.
pub(crate) fn results(
    query: &str,
    hits: Vec<SearchHit>,
    forest: &ForestProjection,
) -> (Vec<(Group, PaletteItem)>, usize) {
    let query = query.trim();
    let mut items = Vec::new();
    if query.is_empty() {
        let tree = forest.current().and_then(|id| forest.tree_of(id));
        let mut stack: Vec<_> = tree.map(|tree| tree.root()).into_iter().collect();
        while let Some(id) = stack.pop() {
            if forest
                .visit(id)
                .is_some_and(|v| v.state == VisitState::Queued)
            {
                items.push((Group::Pending, PaletteItem::Page(id)));
            }
            stack.extend(forest.children(id).iter().rev());
        }
        items.truncate(GROUP_LIMIT);
        return (items, 0);
    }
    let (pages, saved): (Vec<_>, Vec<_>) = hits.into_iter().partition(|h| h.capture_id.is_none());
    items.extend(
        pages
            .into_iter()
            .filter(|hit| forest.visit(hit.visit).is_some())
            .take(GROUP_LIMIT)
            .map(|hit| (Group::Forest, PaletteItem::Page(hit.visit))),
    );
    items.extend(saved.into_iter().take(GROUP_LIMIT).map(|hit| {
        (
            Group::Saved,
            PaletteItem::Saved {
                capture: hit.capture_id.unwrap(),
                title: hit.title,
                excerpt: hit.excerpt,
            },
        )
    }));
    let mut first = 0;
    match destination(query) {
        Ok(Destination::Address(url)) => {
            first = items.len();
            items.push((Group::Elsewhere, PaletteItem::Go(url)));
            items.push((
                Group::Elsewhere,
                PaletteItem::WebSearch {
                    url: web_search(query),
                    query: query.to_owned(),
                },
            ));
        }
        Ok(Destination::Search(url)) => items.push((
            Group::Elsewhere,
            PaletteItem::WebSearch {
                url,
                query: query.to_owned(),
            },
        )),
        Err(_) => {}
    }
    (items, first)
}

/// What running `item` asks for. Alt+Enter plants a new tree instead of a branch.
pub(crate) fn intent(
    item: &PaletteItem,
    scope: PaletteScope,
    new_tree: bool,
    forest: &ForestProjection,
) -> Option<ChromeIntent> {
    let target = match (new_tree, scope) {
        (true, _) | (_, PaletteScope::NewTree) => NavigationTarget::Root,
        (false, PaletteScope::From(parent)) => NavigationTarget::ChildOf(parent),
    };
    Some(match item {
        PaletteItem::Page(visit) if new_tree => ChromeIntent::Navigate {
            url: forest.visit(*visit)?.url.clone(),
            target: NavigationTarget::Root,
        },
        PaletteItem::Page(visit) => ChromeIntent::Restore(*visit),
        PaletteItem::Saved { capture, .. } => ChromeIntent::ReadCapture(*capture),
        PaletteItem::Go(url) | PaletteItem::WebSearch { url, .. } => ChromeIntent::Navigate {
            url: url.clone(),
            target,
        },
    })
}

type Search = Box<dyn Fn(&str) -> Vec<SearchHit>>;

pub(super) enum PaletteEvent {
    Run(ChromeIntent),
    Closed,
}

pub(super) struct Palette {
    pub root: gtk::EventBox,
    panel: gtk::Box,
    context: gtk::Box,
    entry: gtk::Entry,
    list: gtk::ListBox,
    scroll: gtk::ScrolledWindow,
    favicons: Rc<Favicons>,
    page: gtk::Widget,
    search: Search,
    on_event: Box<dyn Fn(PaletteEvent)>,
    generation: Cell<u64>,
    state: RefCell<State>,
}

struct State {
    scope: PaletteScope,
    forest: Option<Rc<ForestProjection>>,
    items: Vec<PaletteItem>,
    rows: Vec<(gtk::ListBoxRow, Option<usize>)>,
    selected: usize,
    shown: u64,
    return_focus: Option<gtk::Widget>,
}

impl Palette {
    pub fn new(
        favicons: Rc<Favicons>,
        page: gtk::Widget,
        search: impl Fn(&str) -> Vec<SearchHit> + 'static,
        on_event: impl Fn(PaletteEvent) + 'static,
    ) -> Rc<Self> {
        let root = gtk::EventBox::new();
        root.set_widget_name("palette");
        root.set_no_show_all(true);
        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.style_context().add_class("palette-box");
        panel.set_halign(gtk::Align::Center);
        panel.set_valign(gtk::Align::Start);
        panel.set_margin_top(70);
        panel.set_size_request(620, -1);
        let context = gtk::Box::new(gtk::Orientation::Horizontal, 7);
        context.style_context().add_class("palette-context");
        panel.pack_start(&context, false, false, 0);
        let input = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        input.set_margin_start(14);
        input.set_margin_end(14);
        input.pack_start(&icon(Icon::Search, 17), false, false, 0);
        let entry = gtk::Entry::new();
        entry.set_widget_name("palette-entry");
        entry.style_context().add_class("palette-entry");
        entry.set_placeholder_text(Some("Search your forest or go to an address…"));
        entry.set_has_frame(false);
        input.pack_start(&entry, true, true, 0);
        panel.pack_start(&input, false, false, 0);
        let list = gtk::ListBox::new();
        list.set_widget_name("palette-results");
        list.style_context().add_class("palette-results");
        list.set_selection_mode(gtk::SelectionMode::None);
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(390);
        scroll.add(&list);
        panel.pack_start(&scroll, false, false, 0);
        let foot = gtk::Label::new(Some(
            "↑↓ choose     Enter open     Alt+Enter in a new tree     Esc close",
        ));
        foot.set_xalign(0.);
        foot.style_context().add_class("palette-foot");
        panel.pack_start(&foot, false, false, 0);
        root.add(&panel);
        panel.show_all();
        let palette = Rc::new(Self {
            root,
            panel,
            context,
            entry,
            list,
            scroll,
            favicons,
            page,
            search: Box::new(search),
            on_event: Box::new(on_event),
            generation: Cell::new(0),
            state: RefCell::new(State {
                scope: PaletteScope::NewTree,
                forest: None,
                items: Vec::new(),
                rows: Vec::new(),
                selected: 0,
                shown: 0,
                return_focus: None,
            }),
        });
        palette.connect();
        palette
    }

    fn connect(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.entry.connect_changed(move |_| {
            let Some(palette) = weak.upgrade() else {
                return;
            };
            let generation = palette.generation.get() + 1;
            palette.generation.set(generation);
            let weak = Rc::downgrade(&palette);
            glib::timeout_add_local_once(Duration::from_millis(40), move || {
                if let Some(palette) = weak.upgrade() {
                    if palette.generation.get() == generation {
                        palette.refresh();
                    }
                }
            });
        });
        let weak = Rc::downgrade(self);
        self.entry.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |palette| palette.key(event))
        });
        let weak = Rc::downgrade(self);
        self.list.connect_row_activated(move |_, row| {
            let Some(palette) = weak.upgrade() else {
                return;
            };
            let index = palette
                .state
                .borrow()
                .rows
                .iter()
                .find(|(r, _)| r == row)
                .and_then(|(_, item)| *item);
            if let Some(index) = index {
                palette.run(index, false);
            }
        });
        let weak = Rc::downgrade(self);
        self.root.connect_button_press_event(move |root, event| {
            let Some(palette) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let (x, y) = event.position();
            let a = palette.panel.allocation();
            let inside = x >= f64::from(a.x())
                && x <= f64::from(a.x() + a.width())
                && y >= f64::from(a.y())
                && y <= f64::from(a.y() + a.height());
            if event.window() == root.window() && !inside {
                palette.close();
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
    }

    pub fn is_open(&self) -> bool {
        self.root.is_visible()
    }

    pub fn present(&self, forest: Rc<ForestProjection>) {
        self.state.borrow_mut().forest = Some(forest);
    }

    pub fn open(&self, scope: PaletteScope, seed: PaletteSeed) {
        let Some(forest) = self.state.borrow().forest.clone() else {
            return;
        };
        if !self.is_open() {
            self.state.borrow_mut().return_focus = self
                .root
                .toplevel()
                .and_then(|w| w.downcast::<gtk::Window>().ok())
                .and_then(|w| w.focused_widget());
        }
        self.state.borrow_mut().scope = scope;
        for child in self.context.children() {
            self.context.remove(&child);
        }
        match scope {
            PaletteScope::From(id) if forest.visit(id).is_some() => {
                let visit = forest.visit(id).unwrap();
                self.context
                    .pack_start(&gtk::Label::new(Some("From")), false, false, 0);
                let surface = self.favicons.surface(visit, 12, self.root.scale_factor());
                self.context
                    .pack_start(&gtk::Image::from_surface(Some(&surface)), false, false, 0);
                let title = gtk::Label::new(Some(visit.label()));
                title.set_ellipsize(pango::EllipsizeMode::End);
                self.context.pack_start(&title, false, false, 0);
            }
            _ => {
                let tag = gtk::Label::new(Some("New tree"));
                tag.style_context().add_class("palette-tag");
                self.context.pack_start(&tag, false, false, 0);
            }
        }
        self.context.show_all();
        let text = match seed {
            PaletteSeed::Empty => String::new(),
            PaletteSeed::CurrentAddress => forest
                .current()
                .and_then(|id| forest.visit(id))
                .map(|visit| visit.url.to_string())
                .unwrap_or_default(),
        };
        self.entry.set_text(&text);
        self.root.show();
        self.entry.grab_focus();
        self.entry.select_region(0, -1);
        self.refresh();
    }

    pub fn close(&self) {
        if !self.is_open() {
            return;
        }
        self.root.hide();
        let focus = self.state.borrow_mut().return_focus.take();
        focus
            .filter(|w| w.is_mapped() && w.is_sensitive())
            .unwrap_or_else(|| self.page.clone())
            .grab_focus();
        (self.on_event)(PaletteEvent::Closed);
    }

    fn refresh(&self) {
        let query = self.entry.text().to_string();
        let hits = if query.trim().is_empty() {
            Vec::new()
        } else {
            (self.search)(&query)
        };
        let Some(forest) = self.state.borrow().forest.clone() else {
            return;
        };
        let (items, first) = results(&query, hits, &forest);
        for row in self.list.children() {
            self.list.remove(&row);
        }
        let scale = self.root.scale_factor();
        let mut rows = Vec::new();
        let mut group = None;
        for (index, (item_group, item)) in items.iter().enumerate() {
            if group != Some(*item_group) {
                group = Some(*item_group);
                let label = gtk::Label::new(Some(item_group.title()));
                label.set_xalign(0.);
                label.style_context().add_class("palette-group");
                let row = gtk::ListBoxRow::new();
                row.set_activatable(false);
                row.set_selectable(false);
                row.add(&label);
                self.list.add(&row);
                rows.push((row, None));
            }
            let row = gtk::ListBoxRow::new();
            row.set_widget_name(match item {
                PaletteItem::Page(_) => "palette-page",
                PaletteItem::Saved { .. } => "palette-saved",
                PaletteItem::Go(_) => "palette-go",
                PaletteItem::WebSearch { .. } => "palette-search",
            });
            row.add(&self.item_widget(item, &forest, scale));
            self.list.add(&row);
            rows.push((row, Some(index)));
        }
        self.list.show_all();
        let mut state = self.state.borrow_mut();
        state.items = items.into_iter().map(|(_, item)| item).collect();
        state.rows = rows;
        state.shown = self.generation.get();
        drop(state);
        self.select(first);
    }

    fn item_widget(&self, item: &PaletteItem, forest: &ForestProjection, scale: i32) -> gtk::Box {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.style_context().add_class("palette-item");
        let (title, detail) = match item {
            PaletteItem::Page(id) => {
                let visit = forest.visit(*id).unwrap();
                let surface = self.favicons.surface(visit, 16, scale);
                content.pack_start(&gtk::Image::from_surface(Some(&surface)), false, false, 0);
                let tree = forest
                    .tree_of(*id)
                    .and_then(|tree| forest.trees().iter().find(|t| t.id == tree))
                    .map(|tree| tree.title.clone())
                    .unwrap_or_default();
                let pending = if visit.state == VisitState::Queued {
                    " · pending"
                } else {
                    ""
                };
                (visit.label().to_owned(), format!("{tree}{pending}"))
            }
            PaletteItem::Saved { title, excerpt, .. } => {
                content.pack_start(&icon(Icon::Search, 15), false, false, 0);
                let excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
                (title.clone(), excerpt.chars().take(140).collect())
            }
            PaletteItem::Go(url) => {
                content.pack_start(&icon(Icon::Globe, 15), false, false, 0);
                (format!("Go to {url}"), String::new())
            }
            PaletteItem::WebSearch { query, .. } => {
                content.pack_start(&icon(Icon::Search, 15), false, false, 0);
                (
                    format!("Search the web for “{query}”"),
                    "DuckDuckGo".to_owned(),
                )
            }
        };
        let title = gtk::Label::new(Some(&title));
        title.set_xalign(0.);
        title.set_ellipsize(pango::EllipsizeMode::End);
        title.style_context().add_class("palette-title");
        content.pack_start(&title, false, true, 0);
        if !detail.is_empty() {
            let detail = gtk::Label::new(Some(&detail));
            detail.set_xalign(0.);
            detail.set_ellipsize(pango::EllipsizeMode::End);
            detail.style_context().add_class("palette-detail");
            content.pack_start(&detail, true, true, 0);
        }
        content
    }

    fn select(&self, index: usize) {
        let mut state = self.state.borrow_mut();
        if state.items.is_empty() {
            return;
        }
        state.selected = index.min(state.items.len() - 1);
        let selected = state.selected;
        for (row, item) in &state.rows {
            let Some(item) = item else { continue };
            if let Some(content) = row.child() {
                if *item == selected {
                    content.style_context().add_class("selected");
                    let a = row.allocation();
                    if a.height() > 1 {
                        let adjustment = self.scroll.vadjustment();
                        adjustment.clamp_page(f64::from(a.y()), f64::from(a.y() + a.height()));
                    }
                } else {
                    content.style_context().remove_class("selected");
                }
            }
        }
    }

    fn key(&self, event: &gdk::EventKey) -> glib::Propagation {
        let key = event.keyval();
        let alt = event.state().contains(gdk::ModifierType::MOD1_MASK);
        let selected = self.state.borrow().selected;
        if key == gdk::keys::constants::Down {
            self.select(selected + 1);
        } else if key == gdk::keys::constants::Up {
            self.select(selected.saturating_sub(1));
        } else if key == gdk::keys::constants::Return || key == gdk::keys::constants::KP_Enter {
            if self.state.borrow().shown != self.generation.get() {
                self.refresh();
            }
            let selected = self.state.borrow().selected;
            self.run(selected, alt);
        } else if key == gdk::keys::constants::Escape {
            self.close();
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    }

    fn run(&self, index: usize, new_tree: bool) {
        let state = self.state.borrow();
        let intent = state
            .forest
            .as_ref()
            .and_then(|forest| intent(state.items.get(index)?, state.scope, new_tree, forest));
        drop(state);
        let Some(intent) = intent else { return };
        self.close();
        (self.on_event)(PaletteEvent::Run(intent));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState, Visit};

    fn forest() -> (ForestProjection, VisitId, VisitId) {
        let id = || -> VisitId { ulid::Ulid::new().to_string().parse().unwrap() };
        let (root, queued) = (id(), id());
        let visit = |id, parent, state: VisitState, at| Visit {
            id,
            parent,
            url: Url::parse("https://example.org/page").unwrap(),
            title: "Example".into(),
            created_at: at,
            opened_at: (state == VisitState::Opened).then_some(at),
            state,
            source: None,
        };
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![
                    visit(root, None, VisitState::Opened, 1),
                    visit(queued, Some(root), VisitState::Queued, 2),
                ],
                current: Some(root),
            },
            vec![TreeState {
                root: TreeId(root),
                last_visit: root,
            }],
        )
        .unwrap();
        (forest, root, queued)
    }

    fn hit(visit: VisitId, capture: Option<i64>) -> SearchHit {
        SearchHit {
            visit,
            capture_id: capture,
            title: "Saved".into(),
            excerpt: "amber otter".into(),
            captured_at: capture,
            source: None,
            state: VisitState::Opened,
        }
    }

    #[test]
    fn pages_saved_text_then_address_then_web_search() {
        let (forest, root, _) = forest();
        let (items, first) = results(
            "example.org",
            vec![hit(root, Some(7)), hit(root, None)],
            &forest,
        );
        let kinds: Vec<_> = items.iter().map(|(group, _)| *group).collect();
        assert_eq!(
            kinds,
            [
                Group::Forest,
                Group::Saved,
                Group::Elsewhere,
                Group::Elsewhere
            ]
        );
        assert_eq!(items[0].1, PaletteItem::Page(root));
        assert!(matches!(items[1].1, PaletteItem::Saved { capture: 7, .. }));
        assert_eq!(
            items[2].1,
            PaletteItem::Go(Url::parse("https://example.org/").unwrap())
        );
        assert!(matches!(items[3].1, PaletteItem::WebSearch { .. }));
        assert_eq!(first, 2, "Enter goes to a typed address");
    }

    #[test]
    fn words_only_offer_a_web_search_and_select_the_first_hit() {
        let (forest, root, _) = forest();
        let (items, first) = results("otter", vec![hit(root, None)], &forest);
        assert_eq!(items.len(), 2);
        assert_eq!(first, 0);
        let (items, _) = results("javascript:alert(1)", vec![], &forest);
        assert!(items.is_empty());
    }

    #[test]
    fn an_empty_query_lists_pending_pages_of_the_current_tree() {
        let (forest, _, queued) = forest();
        let (items, _) = results("  ", vec![], &forest);
        assert_eq!(items, [(Group::Pending, PaletteItem::Page(queued))]);
    }

    #[test]
    fn running_an_item_respects_scope_and_alt_enter() {
        let (forest, root, _) = forest();
        let go = PaletteItem::Go(Url::parse("https://example.org/next").unwrap());
        let branch = intent(&go, PaletteScope::From(root), false, &forest).unwrap();
        assert!(matches!(
            branch,
            ChromeIntent::Navigate { target: NavigationTarget::ChildOf(p), .. } if p == root
        ));
        let planted = intent(&go, PaletteScope::From(root), true, &forest).unwrap();
        assert!(matches!(
            planted,
            ChromeIntent::Navigate {
                target: NavigationTarget::Root,
                ..
            }
        ));
        let saved = PaletteItem::Saved {
            capture: 3,
            title: String::new(),
            excerpt: String::new(),
        };
        assert!(matches!(
            intent(&saved, PaletteScope::NewTree, false, &forest),
            Some(ChromeIntent::ReadCapture(3))
        ));
        assert!(matches!(
            intent(&PaletteItem::Page(root), PaletteScope::NewTree, false, &forest),
            Some(ChromeIntent::Restore(id)) if id == root
        ));
    }
}
