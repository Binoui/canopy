use std::{
    cell::{Cell, RefCell},
    collections::{hash_map::Entry, HashMap},
    rc::{Rc, Weak},
    time::{Duration, Instant},
};

use gtk::{cairo, gdk, pango, prelude::*};

use super::{
    favicon::Favicons,
    graph::{self, EdgeKind, NodeKind, Rect, StripLayout, OPEN_HEIGHT, REST_HEIGHT, TRUNK_Y},
    theme::{Shade, Token, FONT},
};
use crate::{
    browser::{Visit, VisitId, VisitState},
    forest::ForestProjection,
};

const MOVE: Duration = Duration::from_millis(300);
const RESIZE: Duration = Duration::from_millis(220);
const ACCESSORY: f64 = 22.;
const FADE: f64 = 34.;

pub(super) enum StripEvent {
    Enter,
    Leave,
    EnterNode(VisitId),
    LeaveNode,
    Activate(VisitId),
    Menu(bool),
}

pub(super) struct Strip {
    pub root: gtk::EventBox,
    area: gtk::DrawingArea,
    decor: gtk::Box,
    fixed: gtk::Fixed,
    controls: gtk::Fixed,
    accessory: gtk::Widget,
    overflow: gtk::Popover,
    overflow_rows: gtk::Box,
    favicons: Rc<Favicons>,
    shade: Cell<Shade>,
    last_width: Cell<f64>,
    ticking: Cell<bool>,
    state: RefCell<State>,
    on_event: Box<dyn Fn(StripEvent)>,
}

#[derive(Default)]
struct State {
    forest: Option<Rc<ForestProjection>>,
    open: bool,
    target: StripLayout,
    shown: HashMap<VisitId, Shown>,
    height: f64,
    pan: f64,
    manual_pan: Option<f64>,
    motion: Option<Motion>,
    nodes: HashMap<VisitId, Node>,
    badges: HashMap<VisitId, Badge>,
}

struct Badge {
    button: gtk::Button,
    hidden: Rc<RefCell<Vec<VisitId>>>,
}

#[derive(Clone, Copy)]
struct Shown {
    rect: Rect,
    opacity: f64,
}

struct Motion {
    start: Instant,
    from: HashMap<VisitId, Shown>,
    from_height: f64,
    from_pan: f64,
    to_pan: f64,
}

struct Node {
    button: gtk::Button,
    content: gtk::Box,
    image: gtk::Image,
    labels: Option<(gtk::Label, gtk::Label)>,
    look: Option<Look>,
}

#[derive(Clone, PartialEq)]
struct Look {
    kind: NodeKind,
    icons: u64,
    icon: i32,
    pending: bool,
    small: bool,
    label: String,
}

impl Strip {
    pub fn new(
        favicons: Rc<Favicons>,
        accessory: &impl IsA<gtk::Widget>,
        on_event: impl Fn(StripEvent) + 'static,
    ) -> Rc<Self> {
        let root = gtk::EventBox::new();
        root.set_widget_name("strip");
        let overlay = gtk::Overlay::new();
        let area = gtk::DrawingArea::new();
        overlay.add(&area);
        let fixed = gtk::Fixed::new();
        overlay.add_overlay(&fixed);
        // Without a window of its own, so pointer input passes through to the nodes.
        let decor = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        overlay.add_overlay(&decor);
        overlay.set_overlay_pass_through(&decor, true);
        // Badges and the pill's accessory sit above the nodes. Only their own windows take
        // input, so the layer never needs restacking while a click is in flight.
        let controls = gtk::Fixed::new();
        overlay.add_overlay(&controls);
        overlay.set_overlay_pass_through(&controls, true);
        root.add(&overlay);
        let accessory = accessory.as_ref().clone();
        controls.put(&accessory, 0, 0);
        let overflow = gtk::Popover::new(None::<&gtk::Widget>);
        overflow.set_widget_name("overflow-list");
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let heading = gtk::Label::new(Some("More pages here"));
        heading.set_xalign(0.);
        heading.style_context().add_class("menu-heading");
        list.pack_start(&heading, false, false, 0);
        let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        scroll.set_propagate_natural_height(true);
        scroll.set_max_content_height(320);
        let overflow_rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
        scroll.add(&overflow_rows);
        list.pack_start(&scroll, false, false, 0);
        list.set_size_request(300, -1);
        list.show_all();
        overflow.add(&list);
        let strip = Rc::new(Self {
            root,
            area,
            decor,
            fixed,
            controls,
            accessory,
            overflow,
            overflow_rows,
            favicons,
            shade: Cell::new(Shade::Light),
            last_width: Cell::new(0.),
            ticking: Cell::new(false),
            state: RefCell::new(State {
                height: REST_HEIGHT,
                ..State::default()
            }),
            on_event: Box::new(on_event),
        });
        strip.connect();
        strip
    }

    fn connect(self: &Rc<Self>) {
        self.root.add_events(
            gdk::EventMask::ENTER_NOTIFY_MASK
                | gdk::EventMask::LEAVE_NOTIFY_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK,
        );
        let weak = Rc::downgrade(self);
        self.root.connect_enter_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                if let Some(strip) = weak.upgrade() {
                    (strip.on_event)(StripEvent::Enter);
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.root.connect_leave_notify_event(move |_, event| {
            if event.detail() != gdk::NotifyType::Inferior {
                if let Some(strip) = weak.upgrade() {
                    (strip.on_event)(StripEvent::Leave);
                }
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        self.root.connect_scroll_event(move |_, event| {
            if let Some(strip) = weak.upgrade() {
                strip.scroll(event);
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.root.connect_key_press_event(move |_, event| {
            weak.upgrade()
                .map_or(glib::Propagation::Proceed, |strip| strip.key(event))
        });
        let weak = Rc::downgrade(self);
        self.area.connect_draw(move |area, cr| {
            if let Some(strip) = weak.upgrade() {
                strip.paint_edges(area, cr);
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.decor.connect_draw(move |area, cr| {
            if let Some(strip) = weak.upgrade() {
                strip.paint_decor(area, cr);
            }
            glib::Propagation::Stop
        });
        let weak = Rc::downgrade(self);
        self.overflow.connect_closed(move |_| {
            if let Some(strip) = weak.upgrade() {
                (strip.on_event)(StripEvent::Menu(false));
            }
        });
        let weak = Rc::downgrade(self);
        self.root.connect_size_allocate(move |_, allocation| {
            if let Some(strip) = weak.upgrade() {
                let width = f64::from(allocation.width());
                let stale = strip.state.borrow().target.width > 0.
                    && strip.last_width.replace(width) != width;
                if stale {
                    glib::idle_add_local_once({
                        let weak = Rc::downgrade(&strip);
                        move || {
                            if let Some(strip) = weak.upgrade() {
                                strip.relayout(false);
                            }
                        }
                    });
                }
            }
        });
    }

    pub fn set_shade(&self, shade: Shade) {
        self.shade.set(shade);
        self.area.queue_draw();
        self.decor.queue_draw();
    }

    pub fn present(self: &Rc<Self>, forest: Rc<ForestProjection>) {
        let mut state = self.state.borrow_mut();
        state.forest = Some(forest);
        state.manual_pan = None;
        drop(state);
        self.relayout(true);
    }

    pub fn set_open(self: &Rc<Self>, open: bool) {
        let mut state = self.state.borrow_mut();
        if state.open == open {
            return;
        }
        state.open = open;
        state.manual_pan = None;
        drop(state);
        self.relayout(true);
    }

    /// The strip keeps its open allocation until the closing animation ends: 220 ms for
    /// the height, 300 ms for the nodes still moving inside it. A click in that band
    /// during the close is swallowed; the design accepts this rather than letting clicks
    /// fall through a strip that is still drawn.
    pub fn allocation_height(&self) -> i32 {
        self.state.borrow().allocation_height()
    }

    /// Target rectangle of a node in strip coordinates, after the current animation.
    pub fn node_rect(&self, id: VisitId) -> Option<gdk::Rectangle> {
        let state = self.state.borrow();
        let rect = state.target.node(id)?.rect;
        let pan = state
            .manual_pan
            .unwrap_or_else(|| graph::pan(&state.target, self.width()));
        Some(gdk::Rectangle::new(
            (rect.x - rect.w / 2. + pan).round() as i32,
            (rect.y - rect.s / 2.).round() as i32,
            rect.w.round() as i32,
            rect.s.round() as i32,
        ))
    }

    pub fn refresh_icons(&self) {
        let forest = self.state.borrow().forest.clone();
        if let Some(forest) = forest {
            self.restyle(&forest);
        }
    }

    pub fn focus_current(&self) -> bool {
        let state = self.state.borrow();
        let current = state.forest.as_ref().and_then(|forest| forest.current());
        let button = current
            .and_then(|id| state.nodes.get(&id))
            .map(|n| n.button.clone());
        drop(state);
        button.is_some_and(|button| {
            button.grab_focus();
            true
        })
    }

    pub fn has_focus_inside(&self) -> bool {
        self.state
            .borrow()
            .nodes
            .values()
            .any(|node| node.button.has_focus())
    }

    fn width(&self) -> f64 {
        f64::from(self.root.allocated_width().max(1))
    }

    fn relayout(self: &Rc<Self>, animate: bool) {
        let mut state = self.state.borrow_mut();
        let Some(forest) = state.forest.clone() else {
            return;
        };
        let target = match forest.current() {
            Some(current) => {
                let pill = self.pill_width(forest.visit(current).unwrap());
                graph::strip_layout(&forest, current, self.width(), state.open, pill)
            }
            None => StripLayout {
                height: if state.open { OPEN_HEIGHT } else { REST_HEIGHT },
                ..StripLayout::default()
            },
        };
        let to_pan = state
            .manual_pan
            .unwrap_or_else(|| graph::pan(&target, self.width()));
        let parents: HashMap<VisitId, VisitId> = target
            .edges
            .iter()
            .map(|edge| (edge.to, edge.from))
            .collect();
        let mut from = HashMap::with_capacity(target.nodes.len());
        for node in &target.nodes {
            let start = state.shown.get(&node.id).copied().or_else(|| {
                let parent = state.shown.get(parents.get(&node.id)?)?;
                Some(Shown {
                    rect: Rect {
                        s: node.rect.s * 0.4,
                        w: node.rect.w * 0.4,
                        ..parent.rect
                    },
                    opacity: 0.,
                })
            });
            from.insert(
                node.id,
                start
                    .filter(|_| animate || state.motion.is_some())
                    .unwrap_or(Shown {
                        rect: node.rect,
                        opacity: if animate { 0. } else { 1. },
                    }),
            );
        }
        state.motion = Some(Motion {
            start: Instant::now(),
            from,
            from_height: state.height,
            from_pan: state.pan,
            to_pan,
        });
        state.target = target;
        let ids: Vec<_> = state.target.nodes.iter().map(|node| node.id).collect();
        state.nodes.retain(|id, node| {
            let keep = forest.visit(*id).is_some();
            if !keep {
                self.fixed.remove(&node.button);
            }
            keep
        });
        for id in ids {
            if let Entry::Vacant(slot) = state.nodes.entry(id) {
                slot.insert(self.node_widget(id));
            }
        }
        let overflowing: HashMap<VisitId, Vec<VisitId>> = state
            .target
            .nodes
            .iter()
            .filter(|node| !node.hidden.is_empty())
            .map(|node| (node.id, node.hidden.clone()))
            .collect();
        let anchor = self.overflow.relative_to();
        state.badges.retain(|id, badge| {
            let keep = overflowing.contains_key(id);
            if !keep {
                if anchor.as_ref() == Some(badge.button.upcast_ref()) {
                    self.overflow.popdown();
                    self.overflow.set_relative_to(None::<&gtk::Widget>);
                }
                self.controls.remove(&badge.button);
            }
            keep
        });
        for (id, hidden) in overflowing {
            if let Entry::Vacant(slot) = state.badges.entry(id) {
                slot.insert(self.badge(id));
            }
            let badge = &state.badges[&id];
            badge.button.set_label(&format!("+{}", hidden.len()));
            set_accessible_name(&badge.button, &format!("{} more pages", hidden.len()));
            *badge.hidden.borrow_mut() = hidden;
        }
        drop(state);
        self.restyle(&forest);
        self.tick(Instant::now());
        self.start_ticking();
    }

    fn pill_width(&self, visit: &Visit) -> f64 {
        let measure = |text: &str, size: f64, weight: pango::Weight| {
            let layout = self.area.create_pango_layout(Some(text));
            let mut font = pango::FontDescription::from_string(FONT);
            font.set_absolute_size(size * f64::from(pango::SCALE));
            font.set_weight(weight);
            layout.set_font_description(Some(&font));
            f64::from(layout.pixel_size().0)
        };
        let text = measure(visit.label(), 12.5, pango::Weight::Medium).max(measure(
            &site_line(visit),
            11.,
            pango::Weight::Normal,
        ));
        (7. + 16. + 8. + text + 16.).min(250.) + ACCESSORY
    }

    fn node_widget(self: &Rc<Self>, id: VisitId) -> Node {
        let button = gtk::Button::new();
        button.set_widget_name(&format!("node-{id}"));
        button.set_focus_on_click(false);
        button.style_context().add_class("node");
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let image = gtk::Image::new();
        content.pack_start(&image, false, false, 0);
        content.set_halign(gtk::Align::Center);
        button.add(&content);
        let weak = Rc::downgrade(self);
        button.connect_clicked(move |_| {
            if let Some(strip) = weak.upgrade() {
                (strip.on_event)(StripEvent::Activate(id));
            }
        });
        let weak = Rc::downgrade(self);
        button.connect_enter_notify_event(move |_, _| {
            if let Some(strip) = weak.upgrade() {
                (strip.on_event)(StripEvent::EnterNode(id));
            }
            glib::Propagation::Proceed
        });
        let weak = Rc::downgrade(self);
        button.connect_leave_notify_event(move |_, _| {
            if let Some(strip) = weak.upgrade() {
                (strip.on_event)(StripEvent::LeaveNode);
            }
            glib::Propagation::Proceed
        });
        self.fixed.put(&button, 0, 0);
        button.show_all();
        Node {
            button,
            content,
            image,
            labels: None,
            look: None,
        }
    }

    fn badge(self: &Rc<Self>, id: VisitId) -> Badge {
        let button = gtk::Button::new();
        button.set_widget_name(&format!("overflow-{id}"));
        button.set_focus_on_click(false);
        button.style_context().add_class("badge");
        let hidden = Rc::new(RefCell::new(Vec::new()));
        let weak = Rc::downgrade(self);
        let list = hidden.clone();
        button.connect_clicked(move |button| {
            if let Some(strip) = weak.upgrade() {
                let hidden = list.borrow().clone();
                strip.show_hidden(button, &hidden);
            }
        });
        self.controls.put(&button, 0, 0);
        button.show_all();
        Badge { button, hidden }
    }

    fn show_hidden(self: &Rc<Self>, anchor: &gtk::Button, hidden: &[VisitId]) {
        let Some(forest) = self.state.borrow().forest.clone() else {
            return;
        };
        for row in self.overflow_rows.children() {
            self.overflow_rows.remove(&row);
        }
        for &id in hidden.iter().take(200) {
            let Some(visit) = forest.visit(id) else {
                continue;
            };
            let row = super::visit_row(&self.favicons, visit, self.root.scale_factor());
            row.set_widget_name("overflow-page");
            let weak: Weak<Self> = Rc::downgrade(self);
            row.connect_clicked(move |_| {
                if let Some(strip) = weak.upgrade() {
                    strip.overflow.popdown();
                    (strip.on_event)(StripEvent::Activate(id));
                }
            });
            self.overflow_rows.pack_start(&row, false, false, 0);
        }
        self.overflow_rows.show_all();
        self.overflow.set_relative_to(Some(anchor));
        (self.on_event)(StripEvent::Menu(true));
        self.overflow.popup();
    }

    fn restyle(&self, forest: &ForestProjection) {
        let mut state = self.state.borrow_mut();
        let scale = self.root.scale_factor();
        let current = forest.current();
        let placed: Vec<_> = state
            .target
            .nodes
            .iter()
            .map(|node| (node.id, node.kind, node.rect.s))
            .collect();
        for (id, kind, size) in placed {
            let Some(visit) = forest.visit(id) else {
                continue;
            };
            let node = state.nodes.get_mut(&id).unwrap();
            let pending = visit.state == VisitState::Queued;
            let look = Look {
                kind,
                icons: self.favicons.revision(),
                icon: icon_size(kind, size),
                pending,
                small: kind == NodeKind::Tile && size < 24.,
                label: format!("{}{}", visit.label(), site_line(visit)),
            };
            if node.look.as_ref() == Some(&look) {
                continue;
            }
            let context = node.button.style_context();
            for class in ["sm", "raw", "bud", "pending", "current"] {
                context.remove_class(class);
            }
            let classes = [
                (look.small, "sm"),
                (kind == NodeKind::Raw, "raw"),
                (kind == NodeKind::Bud, "bud"),
                (pending, "pending"),
                (kind == NodeKind::Pill, "current"),
            ];
            for (on, class) in classes {
                if on {
                    context.add_class(class);
                }
            }
            node.image.set_visible(kind != NodeKind::Bud);
            node.image
                .set_from_surface(Some(&self.favicons.surface(visit, look.icon, scale)));
            node.image.set_opacity(if pending { 0.5 } else { 1. });
            if kind == NodeKind::Pill {
                let (title, site) = node.labels.get_or_insert_with(|| {
                    let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
                    column.set_valign(gtk::Align::Center);
                    let title = gtk::Label::new(None);
                    let site = gtk::Label::new(None);
                    for (label, class) in [(&title, "pill-title"), (&site, "pill-site")] {
                        label.set_xalign(0.);
                        label.set_ellipsize(pango::EllipsizeMode::End);
                        label.set_max_width_chars(1);
                        label.style_context().add_class(class);
                        column.pack_start(label, false, false, 0);
                    }
                    node.content.pack_start(&column, true, true, 0);
                    column.show_all();
                    (title, site)
                });
                title.set_text(visit.label());
                site.set_text(&site_line(visit));
                node.content.set_halign(gtk::Align::Fill);
                node.content.set_margin_end(ACCESSORY as i32 - 10);
                node.button.set_tooltip_text(None);
            } else {
                if let Some((title, _)) = node.labels.take() {
                    if let Some(column) = title.parent() {
                        node.content.remove(&column);
                    }
                }
                node.content.set_halign(gtk::Align::Center);
                node.content.set_margin_end(0);
                node.button.set_tooltip_text(Some(visit.label()));
            }
            let host = visit.url.host_str().unwrap_or_default();
            let mut name = format!("{}, {host}", visit.label());
            if pending {
                name.push_str(", pending");
            }
            if Some(id) == current {
                name.push_str(", current page");
            }
            set_accessible_name(&node.button, &name);
            node.look = Some(look);
        }
        self.accessory.set_visible(current.is_some());
    }

    fn start_ticking(self: &Rc<Self>) {
        if self.ticking.replace(true) {
            return;
        }
        let weak = Rc::downgrade(self);
        self.area.add_tick_callback(move |_, _| {
            let Some(strip) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if strip.tick(Instant::now()) {
                glib::ControlFlow::Continue
            } else {
                strip.ticking.set(false);
                glib::ControlFlow::Break
            }
        });
    }

    /// Samples the animation at `now` and moves the widgets. Returns whether it continues.
    fn tick(&self, now: Instant) -> bool {
        let mut state = self.state.borrow_mut();
        let old_height = state.allocation_height();
        let Some(motion) = state.motion.take() else {
            return false;
        };
        let elapsed = now.saturating_duration_since(motion.start);
        let t = graph::ease_out_cubic(elapsed.as_secs_f64() / MOVE.as_secs_f64());
        let h = graph::ease_out_cubic(elapsed.as_secs_f64() / RESIZE.as_secs_f64());
        state.height = motion.from_height + (state.target.height - motion.from_height) * h;
        state.pan = motion.from_pan + (motion.to_pan - motion.from_pan) * t;
        let mut shown = HashMap::with_capacity(state.target.nodes.len());
        for node in &state.target.nodes {
            let from = motion.from.get(&node.id).copied().unwrap_or(Shown {
                rect: node.rect,
                opacity: 1.,
            });
            shown.insert(
                node.id,
                Shown {
                    rect: from.rect.lerp(node.rect, t),
                    opacity: from.opacity + (1. - from.opacity) * t,
                },
            );
        }
        state.shown = shown;
        let done = elapsed >= MOVE;
        if !done {
            state.motion = Some(motion);
        }
        let resize = old_height != state.allocation_height();
        self.place(&state);
        drop(state);
        if resize {
            self.root.queue_resize();
        }
        if let Some(frame) = self.root.parent() {
            frame.queue_draw();
        }
        self.area.queue_draw();
        self.decor.queue_draw();
        !done
    }

    fn place(&self, state: &State) {
        let pan = state.pan;
        for (id, node) in &state.nodes {
            match state.shown.get(id) {
                Some(shown) => {
                    let r = shown.rect;
                    node.button
                        .set_size_request(r.w.round() as i32, r.s.round() as i32);
                    self.fixed.move_(
                        &node.button,
                        (r.x - r.w / 2. + pan).round() as i32,
                        (r.y - r.s / 2.).round() as i32,
                    );
                    node.button.set_opacity(shown.opacity);
                    node.button.set_visible(true);
                }
                None => node.button.set_visible(false),
            }
        }
        for (id, badge) in &state.badges {
            if let Some(shown) = state.shown.get(id) {
                let r = shown.rect;
                let badge = &badge.button;
                let width = badge.preferred_width().1;
                self.controls.move_(
                    badge,
                    (r.x + r.w / 2. + 7. + pan).round() as i32 - width,
                    (r.y + r.s / 2. + 6. - 14.).round() as i32,
                );
                badge.set_opacity(shown.opacity);
            }
        }
        let pill = state
            .target
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Pill)
            .and_then(|node| state.shown.get(&node.id));
        if let Some(pill) = pill {
            let r = pill.rect;
            self.controls.move_(
                &self.accessory,
                (r.x + r.w / 2. - ACCESSORY + pan).round() as i32,
                (r.y - 9.).round() as i32,
            );
        }
    }

    /// The open strip's drop shadow, drawn below it in the frame's margin.
    pub fn paint_shadow(&self, cr: &cairo::Context, width: i32) {
        let state = self.state.borrow();
        let openness = ((state.height - REST_HEIGHT) / (OPEN_HEIGHT - REST_HEIGHT)).clamp(0., 1.);
        if openness <= 0. {
            return;
        }
        let alpha = if self.shade.get() == Shade::Dark {
            0.5
        } else {
            0.12
        } * openness;
        let top = state.height;
        let gradient = cairo::LinearGradient::new(0., top, 0., top + 22.);
        gradient.add_color_stop_rgba(0., 0., 0., 0., alpha);
        gradient.add_color_stop_rgba(0.35, 0., 0., 0., alpha * 0.35);
        gradient.add_color_stop_rgba(1., 0., 0., 0., 0.);
        let _ = cr.set_source(&gradient);
        let inset = 14. * openness;
        cr.rectangle(inset, top - 2., f64::from(width) - 2. * inset, 24.);
        let _ = cr.fill();
    }

    fn paint_edges(&self, area: &gtk::DrawingArea, cr: &cairo::Context) {
        let state = self.state.borrow();
        let shade = self.shade.get();
        let width = f64::from(area.allocated_width());
        let height = state.height;
        let openness = ((height - REST_HEIGHT) / (OPEN_HEIGHT - REST_HEIGHT)).clamp(0., 1.);
        let radius = 12. * openness;
        rounded_bottom(cr, width, height, radius);
        shade.source(cr, Token::Bg);
        let _ = cr.fill_preserve();
        shade.source_alpha(cr, Token::Chrome, openness);
        let _ = cr.fill();
        shade.source(cr, Token::Border);
        cr.rectangle(0., 0., width, 1.);
        let _ = cr.fill();
        if openness < 1. {
            shade.source_alpha(cr, Token::Border, 1. - openness);
            cr.rectangle(0., height - 1., width, 1.);
            let _ = cr.fill();
        }
        cr.translate(state.pan, 0.);
        if let Some(pill) = state
            .target
            .nodes
            .iter()
            .find(|node| node.kind == NodeKind::Pill)
            .and_then(|node| state.shown.get(&node.id))
        {
            let r = pill.rect;
            let x = r.x - r.w / 2. + 16.;
            let reach = 40. + 12. * openness;
            let glow = cairo::RadialGradient::new(x, TRUNK_Y, 0., x, TRUNK_Y, reach);
            let c = shade.rgba(Token::GreenSoft);
            glow.add_color_stop_rgba(0., c.red(), c.green(), c.blue(), c.alpha());
            glow.add_color_stop_rgba(1., c.red(), c.green(), c.blue(), 0.);
            let _ = cr.set_source(&glow);
            cr.rectangle(x - reach, TRUNK_Y - reach, 2. * reach, 2. * reach);
            let _ = cr.fill();
        }
        for edge in &state.target.edges {
            let (Some(a), Some(b)) = (state.shown.get(&edge.from), state.shown.get(&edge.to))
            else {
                continue;
            };
            let [p0, p1, p2, p3] = graph::edge_curve(edge, a.rect, b.rect);
            cr.move_to(p0.0, p0.1);
            cr.curve_to(p1.0, p1.1, p2.0, p2.1, p3.0, p3.1);
            match edge.kind {
                EdgeKind::Path => {
                    shade.source(cr, Token::Green);
                    cr.set_line_width(2.);
                }
                EdgeKind::Pending => {
                    shade.source_alpha(cr, Token::Line, b.opacity);
                    cr.set_line_width(1.3);
                    cr.set_dash(&[3., 3.], 0.);
                }
                EdgeKind::Plain => {
                    shade.source_alpha(cr, Token::Line, b.opacity);
                    cr.set_line_width(1.3);
                }
            }
            let _ = cr.stroke();
            cr.set_dash(&[], 0.);
        }
    }

    fn paint_decor(&self, area: &gtk::Box, cr: &cairo::Context) {
        let state = self.state.borrow();
        let shade = self.shade.get();
        let Some(forest) = state.forest.as_ref() else {
            return;
        };
        for node in &state.target.nodes {
            if node.kind == NodeKind::Bud
                || forest.visit(node.id).map(|v| &v.state) != Some(&VisitState::Queued)
            {
                continue;
            }
            let Some(shown) = state.shown.get(&node.id) else {
                continue;
            };
            let r = shown.rect;
            let (size, inset) = if node.kind == NodeKind::Raw {
                (5., 2.)
            } else {
                (7., 3.)
            };
            let cx = r.x + r.w / 2. + inset - size / 2. + state.pan;
            let cy = r.y - r.s / 2. - inset + size / 2.;
            cr.arc(cx, cy, size / 2. + 2., 0., 2. * std::f64::consts::PI);
            shade.source_alpha(cr, Token::Chrome, shown.opacity);
            let _ = cr.fill();
            cr.arc(cx, cy, size / 2., 0., 2. * std::f64::consts::PI);
            shade.source_alpha(cr, Token::Green, shown.opacity);
            let _ = cr.fill();
        }
        let width = f64::from(area.allocated_width());
        let openness = ((state.height - REST_HEIGHT) / (OPEN_HEIGHT - REST_HEIGHT)).clamp(0., 1.);
        let fill = if openness > 0.5 {
            Token::Chrome
        } else {
            Token::Bg
        };
        let c = shade.rgba(fill);
        let fade = |from: f64, to: f64| {
            let gradient = cairo::LinearGradient::new(from, 0., to, 0.);
            gradient.add_color_stop_rgba(0., c.red(), c.green(), c.blue(), 1.);
            gradient.add_color_stop_rgba(1., c.red(), c.green(), c.blue(), 0.);
            let _ = cr.set_source(&gradient);
            cr.rectangle(from.min(to), 1., FADE, state.height - 2.);
            let _ = cr.fill();
        };
        if state.pan < -1. {
            fade(0., FADE);
        }
        if state.target.width + state.pan > width + 1. {
            fade(width, width - FADE);
        }
    }

    fn scroll(self: &Rc<Self>, event: &gdk::EventScroll) {
        let (dx, dy) = event.delta();
        let delta = match event.direction() {
            gdk::ScrollDirection::Up | gdk::ScrollDirection::Left => -40.,
            gdk::ScrollDirection::Down | gdk::ScrollDirection::Right => 40.,
            _ => (if dx.abs() > dy.abs() { dx } else { dy }) * 40.,
        };
        let mut state = self.state.borrow_mut();
        let width = self.width();
        let min = (width - state.target.width).min(0.);
        let pan = (state.manual_pan.unwrap_or(state.pan) - delta).clamp(min, 0.);
        state.manual_pan = Some(pan);
        state.pan = pan;
        self.place(&state);
        drop(state);
        self.area.queue_draw();
        self.decor.queue_draw();
    }

    fn key(&self, event: &gdk::EventKey) -> glib::Propagation {
        let key = event.keyval();
        let step: isize = if key == gdk::keys::constants::Left {
            -1
        } else if key == gdk::keys::constants::Right {
            1
        } else {
            return glib::Propagation::Proceed;
        };
        let state = self.state.borrow();
        let trunk = state.target.trunk();
        let focused = trunk.iter().position(|id| {
            state
                .nodes
                .get(id)
                .is_some_and(|node| node.button.has_focus())
        });
        let Some(index) = focused else {
            return glib::Propagation::Proceed;
        };
        let next = index
            .saturating_add_signed(step)
            .min(trunk.len().saturating_sub(1));
        let button = state
            .nodes
            .get(&trunk[next])
            .map(|node| node.button.clone());
        drop(state);
        if let Some(button) = button {
            button.grab_focus();
        }
        glib::Propagation::Stop
    }
}

impl State {
    fn allocation_height(&self) -> i32 {
        if self.motion.is_some() && !self.open {
            OPEN_HEIGHT as i32
        } else {
            self.height.ceil() as i32
        }
    }
}

fn icon_size(kind: NodeKind, size: f64) -> i32 {
    match kind {
        NodeKind::Pill => 16,
        _ if size <= 14. => size as i32,
        _ if size >= 30. => 18,
        _ if size >= 24. => 16,
        _ if size >= 20. => 13,
        _ => 12,
    }
}

pub(super) fn site_line(visit: &Visit) -> String {
    let host = visit.url.host_str().unwrap_or_default();
    let host = host.strip_prefix("www.").unwrap_or(host);
    match visit.state {
        VisitState::Queued => format!("{host} · pending"),
        VisitState::Opened => format!(
            "{host} · {}",
            super::ago(visit.opened_at.unwrap_or(visit.created_at))
        ),
    }
}

pub(super) fn set_accessible_name(widget: &impl IsA<gtk::Widget>, name: &str) {
    if let Some(accessible) = widget.accessible() {
        gtk::atk::prelude::AtkObjectExt::set_name(&accessible, name);
    }
}

fn rounded_bottom(cr: &cairo::Context, width: f64, height: f64, radius: f64) {
    use std::f64::consts::FRAC_PI_2;
    cr.new_path();
    cr.move_to(0., 0.);
    cr.line_to(width, 0.);
    cr.arc(width - radius, height - radius, radius, 0., FRAC_PI_2);
    cr.arc(radius, height - radius, radius, FRAC_PI_2, 2. * FRAC_PI_2);
    cr.close_path();
}
