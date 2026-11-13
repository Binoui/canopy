use super::{
    favicon::Favicons,
    graph::{self, Rect},
    strip::set_accessible_name,
    theme::{Shade, Token},
};
use crate::{
    browser::{VisitId, VisitState},
    forest::ForestProjection,
};
use gtk::{cairo, gdk, pango, prelude::*};
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

#[derive(Default)]
pub(super) struct ForestLayout {
    nodes: Vec<(VisitId, Rect)>,
    heads: Vec<(usize, f64)>,
    current_width: f64,
}

fn layout(forest: &ForestProjection, labels: bool, measure: impl Fn(&str) -> f64) -> ForestLayout {
    let mut result = ForestLayout::default();
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let mut order: Vec<_> = (0..forest.trees().len()).collect();
    order.sort_by_key(|&i| Some(forest.trees()[i].id) != current);
    let mut y = 34.;
    for index in order {
        let tree = &forest.trees()[index];
        result.heads.push((index, y));
        y += 78.;
        let mut rows = HashMap::new();
        let mut stack = vec![(tree.id.root(), 0, false)];
        let mut row = 0.;
        let mut width: f64 = 0.;
        while let Some((id, depth, visited)) = stack.pop() {
            let children = forest.children(id);
            if !visited && !children.is_empty() {
                stack.push((id, depth, true));
                stack.extend(children.iter().rev().map(|&id| (id, depth + 1, false)));
                continue;
            }
            let r = if children.is_empty() {
                let r = row;
                row += 1.;
                r
            } else {
                (rows[&children[0]] + rows[children.last().unwrap()]) / 2.
            };
            rows.insert(id, r);
            let w = if labels {
                (43. + measure(forest.visit(id).unwrap().label())).clamp(58., 180.)
            } else {
                30.
            };
            let left = 34. + depth as f64 * if labels { 218. } else { 58. };
            result.nodes.push((
                id,
                Rect {
                    x: left + w / 2.,
                    y: y + r * if labels { 40. } else { 38. } + 15.,
                    s: 30.,
                    w,
                },
            ));
            width = width.max(left + w + 34.);
        }
        if result.current_width == 0. {
            result.current_width = width;
        }
        y += row * if labels { 40. } else { 38. } + 34.;
    }
    result
}

#[derive(Clone, Copy)]
struct Camera {
    x: f64,
    y: f64,
    zoom: f64,
}
impl Default for Camera {
    fn default() -> Self {
        Self {
            x: 0.,
            y: 0.,
            zoom: 1.,
        }
    }
}

pub(super) struct ForestView {
    pub root: gtk::Box,
    canvas: gtk::DrawingArea,
    fixed: gtk::Fixed,
    count: gtk::Label,
    labels: gtk::ToggleButton,
    favicons: Rc<Favicons>,
    forest: RefCell<Option<Rc<ForestProjection>>>,
    geometry: RefCell<ForestLayout>,
    camera: Cell<Camera>,
    drag: Cell<Option<(f64, f64, Camera)>>,
    shade: Cell<Shade>,
    tween: Cell<u64>,
    viewport: Cell<(i32, i32)>,
    needs_fit: Cell<bool>,
    on_select: Rc<dyn Fn(VisitId)>,
}

impl ForestView {
    pub fn new(
        favicons: Rc<Favicons>,
        on_select: impl Fn(VisitId) + 'static,
        close: impl Fn() + 'static,
    ) -> Rc<Self> {
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_widget_name("forest-view");
        root.set_no_show_all(true);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.style_context().add_class("forest-toolbar");
        let title = gtk::Label::new(Some("Forest"));
        title.style_context().add_class("card-title");
        toolbar.pack_start(&title, false, false, 0);
        let count = gtk::Label::new(None);
        count.set_xalign(0.);
        count.style_context().add_class("menu-detail");
        toolbar.pack_start(&count, true, true, 0);
        let labels = gtk::ToggleButton::with_label("Titles");
        labels.set_active(true);
        labels.set_widget_name("forest-titles");
        let fit = gtk::Button::with_label("Fit");
        fit.set_widget_name("forest-fit");
        let exit = gtk::Button::with_label("×");
        exit.set_widget_name("forest-close");
        set_accessible_name(&exit, "Close forest");
        for button in [
            labels.clone().upcast::<gtk::Button>(),
            fit.clone(),
            exit.clone(),
        ] {
            button.style_context().add_class("card-action");
            toolbar.pack_start(&button, false, false, 0);
        }
        root.pack_start(&toolbar, false, false, 0);
        let events = gtk::EventBox::new();
        events.set_hexpand(true);
        events.set_vexpand(true);
        events.add_events(
            gdk::EventMask::BUTTON_PRESS_MASK
                | gdk::EventMask::BUTTON_RELEASE_MASK
                | gdk::EventMask::POINTER_MOTION_MASK
                | gdk::EventMask::SCROLL_MASK
                | gdk::EventMask::SMOOTH_SCROLL_MASK,
        );
        let overlay = gtk::Overlay::new();
        let canvas = gtk::DrawingArea::new();
        canvas.set_hexpand(true);
        canvas.set_vexpand(true);
        overlay.add(&canvas);
        let fixed = gtk::Fixed::new();
        overlay.add_overlay(&fixed);
        overlay.set_overlay_pass_through(&fixed, true);
        events.add(&overlay);
        root.pack_start(&events, true, true, 0);
        toolbar.show_all();
        events.show_all();
        let view = Rc::new(Self {
            root,
            canvas,
            fixed,
            count,
            labels,
            favicons,
            forest: RefCell::new(None),
            geometry: RefCell::new(ForestLayout::default()),
            camera: Cell::new(Camera::default()),
            drag: Cell::new(None),
            shade: Cell::new(Shade::Light),
            tween: Cell::new(0),
            viewport: Cell::new((0, 0)),
            needs_fit: Cell::new(false),
            on_select: Rc::new(on_select),
        });
        exit.connect_clicked(move |_| close());
        let w = Rc::downgrade(&view);
        fit.connect_clicked(move |_| {
            if let Some(v) = w.upgrade() {
                v.animate_fit();
            }
        });
        let w = Rc::downgrade(&view);
        view.labels.connect_toggled(move |_| {
            if let Some(v) = w.upgrade() {
                v.relayout();
                v.fit();
            }
        });
        let w = Rc::downgrade(&view);
        view.canvas.connect_draw(move |_, cr| {
            if let Some(v) = w.upgrade() {
                v.paint(cr);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&view);
        view.canvas.connect_size_allocate(move |_, allocation| {
            if let Some(v) = w.upgrade() {
                let size = (allocation.width(), allocation.height());
                let changed = v.viewport.replace(size) != size;
                if size.0 > 1 && v.needs_fit.replace(false) {
                    v.fit();
                } else if changed {
                    v.rebuild();
                }
            }
        });
        let w = Rc::downgrade(&view);
        events.connect_button_press_event(move |_, e| {
            if e.button() == 1 {
                if let Some(v) = w.upgrade() {
                    let (x, y) = e.position();
                    v.drag.set(Some((x, y, v.camera.get())));
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&view);
        events.connect_button_release_event(move |_, _| {
            if let Some(v) = w.upgrade() {
                v.drag.set(None);
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&view);
        events.connect_motion_notify_event(move |_, e| {
            if let Some(v) = w.upgrade() {
                if let Some((x, y, c)) = v.drag.get() {
                    let (px, py) = e.position();
                    v.camera.set(Camera {
                        x: c.x + px - x,
                        y: c.y + py - y,
                        ..c
                    });
                    v.rebuild();
                }
            }
            glib::Propagation::Proceed
        });
        let w = Rc::downgrade(&view);
        events.connect_scroll_event(move |_, e| {
            if let Some(v) = w.upgrade() {
                let (dx, dy) = e.scroll_deltas().unwrap_or_else(|| match e.direction() {
                    gdk::ScrollDirection::Up => (0., -1.),
                    gdk::ScrollDirection::Left => (-1., 0.),
                    gdk::ScrollDirection::Right => (1., 0.),
                    _ => (0., 1.),
                });
                let c = v.camera.get();
                let mut next = c;
                if e.state().contains(gdk::ModifierType::CONTROL_MASK) {
                    let (x, y) = e.position();
                    next.zoom = (c.zoom * (-dy * 0.12).exp()).clamp(0.3, 1.6);
                    let ratio = next.zoom / c.zoom;
                    next.x = x - (x - c.x) * ratio;
                    next.y = y - (y - c.y) * ratio;
                } else {
                    next.x -= dx * 40.;
                    next.y -= dy * 40.;
                }
                v.camera.set(next);
                v.rebuild();
            }
            glib::Propagation::Stop
        });
        view
    }
    pub fn present(&self, forest: Rc<ForestProjection>) {
        self.count.set_text(&format!(
            "{} trees · {} pages",
            forest.trees().len(),
            forest.trees().iter().map(|t| t.page_count).sum::<usize>()
        ));
        *self.forest.borrow_mut() = Some(forest);
        self.relayout();
        if self.root.is_visible() {
            self.rebuild();
        }
    }
    pub fn set_shade(&self, shade: Shade) {
        self.shade.set(shade);
        self.canvas.queue_draw();
    }
    pub fn open(self: &Rc<Self>) {
        self.needs_fit.set(true);
        self.root.show();
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(view) = weak.upgrade() {
                if view.canvas.allocated_width() > 1 && view.needs_fit.replace(false) {
                    view.fit();
                }
            }
        });
    }
    fn animate_fit(self: &Rc<Self>) {
        let from = self.camera.get();
        self.fit();
        let to = self.camera.get();
        let generation = self.tween.get() + 1;
        self.tween.set(generation);
        let start = std::time::Instant::now();
        let weak = Rc::downgrade(self);
        glib::timeout_add_local(std::time::Duration::from_millis(16), move || {
            let Some(view) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if view.tween.get() != generation {
                return glib::ControlFlow::Break;
            }
            let elapsed = start.elapsed().as_secs_f64() / 0.22;
            let t = graph::ease_out_cubic(elapsed);
            view.camera.set(Camera {
                x: from.x + (to.x - from.x) * t,
                y: from.y + (to.y - from.y) * t,
                zoom: from.zoom + (to.zoom - from.zoom) * t,
            });
            view.rebuild();
            if elapsed >= 1. {
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }
    fn relayout(&self) {
        if let Some(f) = self.forest.borrow().as_ref() {
            *self.geometry.borrow_mut() = layout(f, self.labels.is_active(), |text| {
                let l = self.canvas.create_pango_layout(Some(text));
                l.pixel_size().0 as f64
            });
        }
    }
    fn fit(&self) {
        let width = self.geometry.borrow().current_width;
        self.camera.set(Camera {
            zoom: ((self.canvas.allocated_width() as f64 - 20.) / width.max(1.)).clamp(0.3, 1.),
            ..Camera::default()
        });
        self.rebuild();
    }
    fn rebuild(&self) {
        if !self.root.is_visible() {
            return;
        }
        for child in self.fixed.children() {
            self.fixed.remove(&child);
        }
        let forest = self.forest.borrow();
        let Some(forest) = forest.as_ref() else {
            return;
        };
        let c = self.camera.get();
        let geometry = self.geometry.borrow();
        let path: HashSet<_> = forest
            .current()
            .map(|id| forest.path(id).into_iter().collect())
            .unwrap_or_default();
        for &(id, r) in &geometry.nodes {
            let x = c.x + (r.x - r.w / 2.) * c.zoom;
            let y = c.y + (r.y - r.s / 2.) * c.zoom;
            if x + r.w * c.zoom < 0.
                || y + r.s * c.zoom < 0.
                || x > self.canvas.allocated_width() as f64
                || y > self.canvas.allocated_height() as f64
            {
                continue;
            }
            let visit = forest.visit(id).unwrap();
            let button = gtk::Button::new();
            button.set_widget_name(&format!("forest-node-{id}"));
            button.style_context().add_class("node");
            if path.contains(&id) {
                button.style_context().add_class("forest-path");
            }
            if visit.state == VisitState::Queued {
                button.style_context().add_class("pending");
            }
            set_accessible_name(&button, visit.label());
            button.set_tooltip_text(Some(visit.label()));
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let size = (16. * c.zoom).round().max(8.) as i32;
            content.pack_start(
                &gtk::Image::from_surface(Some(&self.favicons.surface(
                    visit,
                    size,
                    self.root.scale_factor(),
                ))),
                false,
                false,
                0,
            );
            if self.labels.is_active() {
                let label = gtk::Label::new(Some(visit.label()));
                label.set_ellipsize(pango::EllipsizeMode::End);
                label.style_context().add_class("pill-title");
                let attributes = pango::AttrList::new();
                attributes.insert(pango::AttrSize::new_size_absolute(
                    (12.5 * c.zoom * pango::SCALE as f64) as i32,
                ));
                label.set_attributes(Some(&attributes));
                label.set_width_chars(1);
                content.pack_start(&label, true, true, 0);
            }
            button.add(&content);
            button.set_size_request((r.w * c.zoom) as i32, (r.s * c.zoom) as i32);
            let on_select = self.on_select.clone();
            button.connect_clicked(move |_| on_select(id));
            self.fixed.put(&button, x as i32, y as i32);
            button.show_all();
        }
        self.canvas.queue_draw();
    }
    fn paint(&self, cr: &cairo::Context) {
        dapple(
            cr,
            self.shade.get(),
            self.canvas.allocated_width(),
            self.canvas.allocated_height(),
        );
        let forest = self.forest.borrow();
        let Some(f) = forest.as_ref() else { return };
        let g = self.geometry.borrow();
        let c = self.camera.get();
        let shade = self.shade.get();
        let _ = cr.save();
        cr.translate(c.x, c.y);
        cr.scale(c.zoom, c.zoom);
        let positions: HashMap<_, _> = g.nodes.iter().copied().collect();
        let path: HashSet<_> = f
            .current()
            .map(|id| f.path(id).into_iter().collect())
            .unwrap_or_default();
        for &(id, b) in &g.nodes {
            let visit = f.visit(id).unwrap();
            if let Some(a) = visit.parent.and_then(|p| positions.get(&p)) {
                let green = path.contains(&id);
                shade.source(cr, if green { Token::Green } else { Token::Line });
                cr.set_line_width(if green { 1.8 } else { 1.2 });
                cr.set_dash(
                    if visit.state == VisitState::Queued {
                        &[4., 4.]
                    } else {
                        &[]
                    },
                    0.,
                );
                let x = a.x + a.w / 2.;
                let end = b.x - b.w / 2.;
                let dx = (end - x) / 2.;
                cr.move_to(x, a.y);
                cr.curve_to(x + dx, a.y, end - dx, b.y, end, b.y);
                let _ = cr.stroke();
            }
        }
        cr.set_dash(&[], 0.);
        for &(index, y) in &g.heads {
            let tree = &f.trees()[index];
            shade.source(cr, Token::Text);
            cr.select_font_face(
                super::theme::FONT,
                cairo::FontSlant::Normal,
                cairo::FontWeight::Bold,
            );
            cr.set_font_size(15.);
            cr.move_to(34., y + 16.);
            let _ = cr.show_text(&tree.title);
            shade.source(cr, Token::Muted);
            cr.select_font_face(
                super::theme::FONT,
                cairo::FontSlant::Normal,
                cairo::FontWeight::Normal,
            );
            cr.set_font_size(11.5);
            cr.move_to(34., y + 36.);
            let current = f.current().and_then(|id| f.tree_of(id)) == Some(tree.id);
            let _ = cr.show_text(&format!(
                "{} pages · {} pending{}",
                tree.page_count,
                tree.pending_count,
                if current { " · Current tree" } else { "" }
            ));
        }
        let _ = cr.restore();
    }
}

pub(super) fn dapple(cr: &cairo::Context, shade: Shade, width: i32, height: i32) {
    shade.source(cr, Token::Bg);
    let _ = cr.paint();
    let green = shade.rgba(Token::Green);
    for (x, y, r) in [
        (0.14, 0.22, 0.26),
        (0.82, 0.12, 0.22),
        (0.64, 0.84, 0.28),
        (0.3, 0.88, 0.18),
    ] {
        let gradient = cairo::RadialGradient::new(
            x * width as f64,
            y * height as f64,
            0.,
            x * width as f64,
            y * height as f64,
            r * width as f64,
        );
        gradient.add_color_stop_rgba(0., green.red(), green.green(), green.blue(), 0.045);
        gradient.add_color_stop_rgba(1., green.red(), green.green(), green.blue(), 0.);
        let _ = cr.set_source(&gradient);
        let _ = cr.paint();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState, Visit};
    fn fixture(trees: usize, depth: usize) -> ForestProjection {
        let mut visits = Vec::new();
        let mut states = Vec::new();
        let mut current = None;
        for _ in 0..trees {
            let root: VisitId = ulid::Ulid::new().to_string().parse().unwrap();
            let mut parent = None;
            for d in 0..depth {
                let id = if d == 0 {
                    root
                } else {
                    ulid::Ulid::new().to_string().parse().unwrap()
                };
                visits.push(Visit {
                    id,
                    parent,
                    url: url::Url::parse("https://example.org/").unwrap(),
                    title: "Example".into(),
                    created_at: visits.len() as i64,
                    opened_at: Some(0),
                    state: VisitState::Opened,
                    source: None,
                });
                parent = Some(id);
            }
            current = parent;
            states.push(TreeState {
                root: TreeId(root),
                last_visit: parent.unwrap(),
            });
        }
        ForestProjection::from_snapshot(Snapshot { visits, current }, states).unwrap()
    }
    #[test]
    fn deep_layout_is_iterative_and_keeps_every_node() {
        let f = fixture(1, 20_000);
        let l = layout(&f, true, |_| 50.);
        assert_eq!(l.nodes.len(), 20_000);
        assert_eq!(l.nodes.first().unwrap().1.x, 34. + 19_999. * 218. + 46.5);
    }
    #[test]
    fn all_trees_are_ordered_current_first_in_both_modes() {
        let f = fixture(1000, 3);
        for labels in [true, false] {
            let l = layout(&f, labels, |_| 50.);
            assert_eq!(l.heads.len(), 1000);
            assert_eq!(l.nodes.len(), 3000);
            assert_eq!(l.heads[0].0, 999);
            assert!(l.heads.windows(2).all(|w| w[0].1 < w[1].1));
            assert_eq!(l.nodes[0].1.w, if labels { 93. } else { 30. });
            assert_eq!(l.current_width, if labels { 597. } else { 214. });
        }
    }
}
