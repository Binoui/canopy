mod pictures;
pub(super) use pictures::response as picture_response;

use std::{
    collections::HashMap,
    rc::{Rc, Weak},
    time::Duration,
};

use pictures::{Picture, Pixels, Sizes};

use crate::{
    browser::VisitId,
    forest::ForestProjection,
    shortcuts::{KeyCombo, Modifiers},
};

use super::waiting::Waiting;

use anyhow::Result;
use gtk::prelude::*;
use serde_json::{json, Value};
use wry::WebViewExtUnix;

use crate::pages::{DismissCompletion, Pages};

use super::{view, ChromeFrame, ChromeIntent, HtmlChrome, Inner, CHROME_HEIGHT};

const WAITING_LIMIT: usize = 5;
pub(super) const READ_LIMIT: usize = 8;

pub(super) struct SwitchChoices {
    pub(super) here: Option<VisitId>,
    pub(super) waiting: Vec<VisitId>,
    pub(super) read: Vec<VisitId>,
    pub(super) more_waiting: usize,
}

impl SwitchChoices {
    pub(super) fn of(forest: &ForestProjection, recent: &[VisitId]) -> Self {
        let waiting = Waiting::of(forest);
        Self {
            here: forest.current(),
            waiting: waiting
                .pages()
                .iter()
                .take(WAITING_LIMIT)
                .map(|v| v.id)
                .collect(),
            read: recent
                .iter()
                .copied()
                .filter(|id| forest.visit(*id).is_some())
                .take(READ_LIMIT)
                .collect(),
            more_waiting: waiting.len().saturating_sub(WAITING_LIMIT),
        }
    }

    pub(super) fn first(&self) -> Option<VisitId> {
        self.waiting.first().or_else(|| self.read.first()).copied()
    }
}

#[derive(Default)]
pub(super) struct Switcher {
    next: u64,
    phase: Phase,
    captures: usize,
    bytes: u64,
    #[cfg(feature = "gui-smoke")]
    snapshots: u64,
}

#[derive(Default)]
enum Phase {
    #[default]
    Closed,
    Armed(Armed),
    Showing(Box<Opening>),
    Released(Released),
}

struct Armed {
    id: u64,
    hold: Modifiers,
    invoked_with: KeyCombo,
    choices: SwitchChoices,
    pages: Weak<Pages>,
    timer: Option<glib::SourceId>,
}

impl Drop for Armed {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.take() {
            timer.remove();
        }
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Kind {
    Here,
    Waiting,
    Read,
}

#[derive(Clone, Copy)]
pub(super) struct Row {
    pub(super) id: VisitId,
    kind: Kind,
}

struct Opening {
    id: u64,
    hold: Modifiers,
    here: Option<VisitId>,
    selected: VisitId,
    rows: Vec<Row>,
    more_waiting: usize,
    focus_was_in_chrome: bool,
    pictures: HashMap<VisitId, Picture>,
    capture_layout: FilmLayout,
    painted: bool,
    dismiss: Option<Dismiss>,
}

struct Released {
    opening: u64,
    here: Option<VisitId>,
    selected: VisitId,
    rows: Vec<Row>,
    dismiss: Dismiss,
}

enum Dismiss {
    Unclaimed(VisitId),
    Pending,
}

impl Dismiss {
    fn claim(&mut self, head: VisitId) -> bool {
        if !matches!(self, Self::Unclaimed(id) if *id == head) {
            return false;
        }
        *self = Self::Pending;
        true
    }
}

impl SwitchChoices {
    pub(super) fn rows(&self) -> impl Iterator<Item = Row> + '_ {
        [
            (Kind::Here, self.here.as_slice()),
            (Kind::Waiting, self.waiting.as_slice()),
            (Kind::Read, self.read.as_slice()),
        ]
        .into_iter()
        .flat_map(|(kind, ids)| ids.iter().map(move |&id| Row { id, kind }))
    }
}

impl Opening {
    fn step(&mut self, back: bool) {
        let index = self
            .rows
            .iter()
            .position(|row| row.id == self.selected)
            .unwrap();
        let count = self.rows.len();
        self.selected = self.rows[(index + if back { count - 1 } else { 1 }) % count].id;
    }
}

impl Switcher {
    fn hold(&self) -> Option<Modifiers> {
        match &self.phase {
            Phase::Armed(armed) => Some(armed.hold),
            Phase::Showing(opening) => Some(opening.hold),
            _ => None,
        }
    }
}

fn modifiers_held(window: &gtk::Window, hold: Modifiers) -> bool {
    gdk::Keymap::for_display(&window.display()).is_some_and(|keymap| hold.are_held(&keymap))
}

impl HtmlChrome {
    pub(crate) fn arm_switcher(
        &self,
        pages: &Rc<Pages>,
        window: &gtk::Window,
        combo: KeyCombo,
    ) -> Result<()> {
        let inner = &self.inner;
        inner.finish_switcher(false);
        let choices = {
            let frame = inner.frame.borrow();
            let Some(frame) = frame.as_ref() else {
                return Ok(());
            };
            let recent = pages.read(|browser| browser.recent_pages(&frame.forest, READ_LIMIT))?;
            SwitchChoices::of(&frame.forest, &recent)
        };
        let Some(first) = choices.first() else {
            self.toast(super::Toast::Info("No other pages in this tree".into()));
            return Ok(());
        };
        let Some(hold) = combo.hold().filter(|&hold| modifiers_held(window, hold)) else {
            inner.open_switch_choice(first);
            return Ok(());
        };
        let id = {
            let mut switcher = inner.switcher.borrow_mut();
            switcher.next += 1;
            switcher.next
        };
        let weak = inner.me.clone();
        let window = window.downgrade();
        let timer = glib::timeout_add_local_once(Duration::from_millis(160), move || {
            let Some(inner) = weak.upgrade() else {
                return;
            };
            {
                let mut switcher = inner.switcher.borrow_mut();
                let Phase::Armed(armed) = &mut switcher.phase else {
                    return;
                };
                if armed.id != id {
                    return;
                }
                armed.timer.take();
            }
            if window
                .upgrade()
                .is_some_and(|window| modifiers_held(&window, hold))
            {
                inner.reveal_switcher(None);
            } else {
                inner.finish_switcher(true);
            }
        });
        inner.switcher.borrow_mut().phase = Phase::Armed(Armed {
            id,
            hold,
            invoked_with: combo,
            choices,
            pages: Rc::downgrade(pages),
            timer: Some(timer),
        });
        Ok(())
    }

    pub(crate) fn step_switcher(&self, back: bool, tab: bool) -> bool {
        let inner = &self.inner;
        let mut switcher = inner.switcher.borrow_mut();
        match &mut switcher.phase {
            Phase::Showing(opening) => {
                opening.step(back);
                inner.paint_switch_selection(opening);
            }
            Phase::Armed(_) if tab => {
                drop(switcher);
                inner.reveal_switcher(Some(back));
            }
            _ => return false,
        }
        true
    }

    pub(crate) fn switcher_active(&self) -> bool {
        self.inner.switcher.borrow().hold().is_some()
    }

    pub(crate) fn switcher_tab(&self, combo: KeyCombo) -> Option<bool> {
        self.inner
            .switcher
            .borrow()
            .hold()
            .and_then(|hold| combo.held_tab_direction(hold))
    }

    pub(crate) fn release_switcher_modifier(&self, key: gdk::keys::Key) {
        let releases = self
            .inner
            .switcher
            .borrow()
            .hold()
            .is_some_and(|hold| hold.released_by(key));
        if releases {
            self.finish_switcher(true);
        }
    }

    pub(crate) fn dismiss_switcher_selection(&self) -> bool {
        let inner = &self.inner;
        let mut switcher = inner.switcher.borrow_mut();
        let Phase::Showing(opening) = &mut switcher.phase else {
            return false;
        };
        let head = opening.selected;
        if opening.dismiss.is_none()
            && opening
                .rows
                .iter()
                .any(|row| row.id == head && row.kind == Kind::Waiting)
        {
            opening.dismiss = Some(Dismiss::Unclaimed(head));
            drop(switcher);
            if let Some(host) = inner.host() {
                host.dispatch(ChromeIntent::Dismiss(head));
            }
        }
        true
    }

    pub(crate) fn dismiss_completion(&self, head: VisitId) -> Option<DismissCompletion> {
        let mut switcher = self.inner.switcher.borrow_mut();
        let (id, request) = match &mut switcher.phase {
            Phase::Showing(opening) => (opening.id, opening.dismiss.as_mut()?),
            Phase::Released(released) => (released.opening, &mut released.dismiss),
            _ => return None,
        };
        if !request.claim(head) {
            return None;
        }
        let weak = self.inner.me.clone();
        Some(Box::new(move |result| {
            if let Some(inner) = weak.upgrade() {
                inner.switcher_dismissed(id, result);
            }
        }))
    }

    pub(crate) fn finish_switcher(&self, open: bool) {
        self.inner.finish_switcher(open);
    }
}

fn survives(row: &Row, forest: &ForestProjection) -> bool {
    match row.kind {
        Kind::Waiting => forest.is_waiting(row.id),
        _ => forest.visit(row.id).is_some(),
    }
}

fn survivor(rows: &[Row], selected: VisitId, forest: &ForestProjection) -> Option<VisitId> {
    let index = rows.iter().position(|row| row.id == selected)?;
    rows.iter()
        .cycle()
        .skip(index)
        .take(rows.len())
        .find(|row| survives(row, forest))
        .map(|row| row.id)
}

impl Opening {
    fn prune(&mut self, forest: &ForestProjection) {
        if let Some(next) = survivor(&self.rows, self.selected, forest) {
            self.selected = next;
        }
        self.rows.retain(|row| {
            if survives(row, forest) {
                return true;
            }
            self.pictures.remove(&row.id);
            false
        });
        let shown = self
            .rows
            .iter()
            .filter(|row| row.kind == Kind::Waiting)
            .count();
        self.more_waiting = Waiting::of(forest).len().saturating_sub(shown);
    }
}

impl Inner {
    fn reveal_switcher(&self, step: Option<bool>) {
        let Phase::Armed(armed) = std::mem::take(&mut self.switcher.borrow_mut().phase) else {
            return;
        };
        let rows: Vec<_> = armed.choices.rows().collect();
        let capture_layout = FilmLayout::of(self.size.get(), &rows, armed.choices.more_waiting);
        let pictures = armed.pages.upgrade().map_or_else(HashMap::new, |pages| {
            rows.iter()
                .map(|row| {
                    let picture = pages
                        .switcher_source(row.id)
                        .map_or(Picture::Unavailable, Picture::Waiting);
                    (row.id, picture)
                })
                .collect()
        });
        let mut opening = Opening {
            id: armed.id,
            hold: armed.hold,
            here: armed.choices.here,
            selected: armed.choices.first().unwrap(),
            rows,
            capture_layout,
            painted: false,
            dismiss: None,
            pictures,
            more_waiting: armed.choices.more_waiting,
            focus_was_in_chrome: self.view.webview().has_focus(),
        };
        if let Some(back) = step {
            opening.step(back);
        }
        let message = {
            let frame = self.frame.borrow();
            let Some(frame) = frame.as_ref() else {
                return;
            };
            let rows: Vec<_> = opening
                .rows
                .iter()
                .map(|&row| {
                    let mut value = row.value(frame);
                    value["pictureStatus"] =
                        if matches!(opening.pictures.get(&row.id), Some(Picture::Waiting(_))) {
                            json!("Preparing preview")
                        } else {
                            json!("Not loaded")
                        };
                    value
                })
                .collect();
            let keys = armed.invoked_with.switcher_footer(armed.hold);
            json!({
                "opening": opening.id.to_string(),
                "selected": opening.selected.to_string(),
                "rows": rows,
                "layout": opening.capture_layout.value(),
                "timing": cfg!(feature = "gui-smoke"),
                "moreWaiting": opening.more_waiting,
                "keys": {
                    "next": keys.next, "previous": keys.previous,
                    "alternate": keys.alternate, "hold": keys.hold,
                },
            })
        };
        self.switcher.borrow_mut().phase = Phase::Showing(Box::new(opening));
        self.cover.set(None);
        self.apply_bounds();
        self.call("switcher", &[json!("show"), message]);
        let _ = self.view.focus();
    }

    pub(super) fn finish_switcher(&self, open: bool) {
        let phase = std::mem::take(&mut self.switcher.borrow_mut().phase);
        let choice = match phase {
            Phase::Closed => return,
            Phase::Released(released) => {
                if open {
                    self.switcher.borrow_mut().phase = Phase::Released(released);
                }
                return;
            }
            Phase::Armed(armed) => armed.choices.first().filter(|_| open),
            Phase::Showing(opening) => {
                let choice = Some(opening.selected).filter(|&id| open && Some(id) != opening.here);
                self.call(
                    "switcher",
                    &[
                        json!("close"),
                        json!({
                            "opening": opening.id.to_string(),
                            "restoreFocus": choice.is_none() && opening.focus_was_in_chrome,
                        }),
                    ],
                );
                if choice.is_none() && !opening.focus_was_in_chrome {
                    if let Some(host) = self.host() {
                        host.focus_page();
                    }
                }
                if let Some(dismiss) = opening.dismiss.filter(|_| choice.is_some()) {
                    self.switcher.borrow_mut().phase = Phase::Released(Released {
                        opening: opening.id,
                        here: opening.here,
                        selected: opening.selected,
                        rows: opening.rows,
                        dismiss,
                    });
                    return;
                }
                choice
            }
        };
        if let Some(id) = choice {
            self.open_switch_choice(id);
        }
    }

    fn open_switch_choice(&self, id: VisitId) {
        self.call("navigated", &[]);
        if let Some(host) = self.host() {
            host.dispatch(ChromeIntent::Restore(id));
            host.focus_page();
        }
    }

    pub(super) fn relayout_switcher(&self) {
        if let Phase::Showing(opening) = &self.switcher.borrow().phase {
            self.call(
                "switcher",
                &[
                    json!("layout"),
                    json!({
                        "opening": opening.id.to_string(),
                        "layout": FilmLayout::of(self.size.get(), &opening.rows, opening.more_waiting).value(),
                    }),
                ],
            );
        }
    }

    pub(super) fn switcher_frame_changed(&self, frame: &ChromeFrame) {
        let mut switcher = self.switcher.borrow_mut();
        let here = match &switcher.phase {
            Phase::Closed => return,
            Phase::Armed(armed) => armed.choices.here,
            Phase::Showing(opening) => opening.here,
            Phase::Released(released) => released.here,
        };
        if here != frame.forest.current() {
            drop(switcher);
            self.finish_switcher(false);
            return;
        }
        if let Phase::Showing(opening) = &mut switcher.phase {
            if opening.dismiss.is_none() {
                opening.prune(&frame.forest);
            }
            self.paint_switcher_rows(opening, frame);
        }
    }

    fn switcher_dismissed(&self, id: u64, mut result: Result<ForestProjection>) {
        let mut switcher = self.switcher.borrow_mut();
        let mut choice = None;
        match &mut switcher.phase {
            Phase::Showing(opening) if opening.id == id => {
                opening.dismiss = None;
                if let Ok(forest) = &mut result {
                    if let Some(here) = opening.here {
                        forest.show_current(here);
                    }
                    opening.prune(forest);
                    if let Some(frame) = self.frame.borrow().as_ref() {
                        self.paint_switcher_rows(opening, frame);
                    }
                }
            }
            Phase::Released(released) if released.opening == id => {
                if let Ok(forest) = &result {
                    choice = survivor(&released.rows, released.selected, forest)
                        .filter(|&visit| Some(visit) != released.here);
                }
                switcher.phase = Phase::Closed;
            }
            _ => {}
        }
        drop(switcher);
        if let Err(error) = result {
            self.report(error);
        }
        if let Some(choice) = choice {
            self.open_switch_choice(choice);
        }
    }

    fn paint_switcher_rows(&self, opening: &Opening, frame: &ChromeFrame) {
        let rows: Vec<_> = opening.rows.iter().map(|&row| row.value(frame)).collect();
        self.call("switcher", &[json!("patch"), json!({
            "opening": opening.id.to_string(),
            "selected": opening.selected.to_string(),
            "rows": rows,
            "layout": FilmLayout::of(self.size.get(), &opening.rows, opening.more_waiting).value(),
            "moreWaiting": opening.more_waiting,
        })]);
    }

    pub(super) fn switcher_pointer(self: &Rc<Self>, message: &Value) {
        let Some(id) = message["opening"].as_str().and_then(|id| id.parse().ok()) else {
            return;
        };
        if message["action"] == "painted" {
            if let Phase::Showing(opening) = &mut self.switcher.borrow_mut().phase {
                if opening.id == id {
                    opening.painted = true;
                }
            }
            self.fill_pictures(id);
            return;
        }
        let Some(target) = message["id"]
            .as_str()
            .and_then(|id| id.parse::<VisitId>().ok())
        else {
            return;
        };
        let mut switcher = self.switcher.borrow_mut();
        let Phase::Showing(opening) = &mut switcher.phase else {
            return;
        };
        if opening.id != id || !opening.rows.iter().any(|row| row.id == target) {
            return;
        }
        match message["action"].as_str() {
            Some("open") => {
                opening.selected = target;
                drop(switcher);
                self.finish_switcher(true);
            }
            Some("hover") if opening.selected != target => {
                opening.selected = target;
                self.paint_switch_selection(opening);
            }
            _ => {}
        }
    }

    fn paint_switch_selection(&self, opening: &Opening) {
        self.call(
            "switcher",
            &[
                json!("select"),
                json!({
                    "opening": opening.id.to_string(), "selected": opening.selected.to_string(),
                }),
            ],
        );
    }
}

impl Row {
    pub(super) fn value(self, frame: &ChromeFrame) -> Value {
        let forest = &frame.forest;
        let Some(visit) = forest.visit(self.id) else {
            return Value::Null;
        };
        let mut value =
            view::visit_value(forest, visit, &frame.loaded, &frame.live, &frame.playing);
        let caption = match self.kind {
            Kind::Here => "You are here".to_owned(),
            Kind::Waiting => visit
                .parent
                .and_then(|parent| forest.visit(parent))
                .map_or_else(
                    || "Waiting".to_owned(),
                    |parent| format!("Waiting, from {}", parent.label()),
                ),
            Kind::Read => format!("Read {}", value["when"].as_str().unwrap_or_default()),
        };
        let name = value["name"].as_str().unwrap_or_default();
        value["name"] = json!(match self.kind {
            Kind::Waiting => name.to_owned(),
            Kind::Here => format!("{name}, current page"),
            Kind::Read => format!("{name}, read"),
        });
        value["site"] = json!(view::short_host(&visit.url));
        value["caption"] = json!(caption);
        value["kind"] = json!(match self.kind {
            Kind::Here => "here",
            Kind::Waiting => "waiting",
            Kind::Read => "read",
        });
        value
    }
}

#[derive(Clone, Copy, Debug)]
struct BoxPx(i32, i32, i32, i32);

struct FilmLayout {
    backdrop: BoxPx,
    stage: Option<BoxPx>,
    caption: BoxPx,
    row: BoxPx,
    run: BoxPx,
    cards: Vec<(VisitId, BoxPx)>,
    divider: Option<BoxPx>,
    more: Option<BoxPx>,
    footer: BoxPx,
    picture_height: i32,
}

const CARD_WIDTH: i32 = 120;
const CARD_GAP: i32 = 10;
const ROW_INSET: i32 = 6;

impl FilmLayout {
    fn of((width, height): (f64, f64), rows: &[Row], more_waiting: usize) -> Self {
        let (w, h) = (width.round() as i32, height.round() as i32);
        let page_h = (h - CHROME_HEIGHT as i32).max(1);
        let picture_height = (f64::from(CARD_WIDTH * page_h) / f64::from(w.max(1)))
            .round()
            .clamp(56., 76.) as i32;
        let row_h = picture_height + 36;
        let side = 16;
        let foot_h = 30;
        let foot_y = (h - 12 - foot_h).max(row_h);
        let row_y = foot_y - row_h;
        let top = (CHROME_HEIGHT as i32).min(row_y.saturating_sub(36)).max(0);
        let caption_y = row_y - 34;
        let max_h = caption_y - top - 23;
        let max_w = (w - 2 * side).max(1);
        let stage = (max_h >= 96).then(|| {
            let scale =
                (f64::from(max_w) / f64::from(w.max(1))).min(f64::from(max_h) / f64::from(page_h));
            let sw = (f64::from(w) * scale).round() as i32;
            let sh = (f64::from(page_h) * scale).round() as i32;
            BoxPx((w - sw) / 2, top + 15 + (max_h - sh) / 2, sw, sh)
        });
        let mut x = ROW_INSET;
        let mut divider = None;
        let has_waiting = rows.iter().any(|row| row.kind == Kind::Waiting);
        let cards: Vec<_> = rows
            .iter()
            .map(|row| {
                if row.kind == Kind::Read && has_waiting && divider.is_none() {
                    divider = Some(BoxPx(x, 6, 1, picture_height + 20));
                    x += CARD_GAP + 1;
                }
                let bounds = BoxPx(x, 6, CARD_WIDTH, picture_height + 22);
                x += CARD_WIDTH + CARD_GAP;
                (row.id, bounds)
            })
            .collect();
        let run_w = x - CARD_GAP + ROW_INSET;
        let more_w = if more_waiting > 0 { 106 } else { 0 };
        Self {
            backdrop: BoxPx(0, top, w, h - top),
            stage,
            caption: BoxPx(side, caption_y.max(0), max_w, 28.min(row_y)),
            row: BoxPx(side, row_y, max_w, row_h),
            run: BoxPx(((max_w - run_w) / 2).max(0), 0, run_w, row_h - 8),
            cards,
            divider,
            more: (more_w > 0).then_some(BoxPx(w - side - more_w, foot_y, more_w, foot_h)),
            footer: BoxPx(side, foot_y, (max_w - more_w).max(1), foot_h),
            picture_height,
        }
    }
}

impl BoxPx {
    fn value(self) -> [i32; 4] {
        [self.0, self.1, self.2, self.3]
    }
}

impl FilmLayout {
    fn value(&self) -> Value {
        json!({
            "backdrop": self.backdrop.value(),
            "stage": self.stage.map(BoxPx::value),
            "caption": self.caption.value(),
            "row": self.row.value(),
            "run": self.run.value(),
            "cards": self.cards.iter().map(|(id, bounds)| (id.to_string(), bounds.value())).collect::<HashMap<_, _>>(),
            "divider": self.divider.map(BoxPx::value),
            "more": self.more.map(BoxPx::value),
            "footer": self.footer.value(),
            "picture_height": self.picture_height,
        })
    }
}

impl Switcher {
    fn showing(&self, id: u64) -> Option<&Opening> {
        match &self.phase {
            Phase::Showing(opening) if opening.id == id => Some(opening),
            _ => None,
        }
    }
}

impl Opening {
    fn next_picture(&self) -> Option<(VisitId, crate::pages::SwitcherSource)> {
        let selected = self
            .rows
            .iter()
            .position(|row| row.id == self.selected)
            .unwrap();
        let mut order = self.rows[selected..]
            .iter()
            .zip(0..)
            .map(|(row, offset)| (offset, row))
            .chain(
                self.rows[..selected]
                    .iter()
                    .rev()
                    .zip(1..)
                    .map(|(row, offset)| (offset, row)),
            )
            .collect::<Vec<_>>();
        order.sort_by_key(|(offset, _)| *offset);
        order
            .into_iter()
            .find_map(|(_, row)| match self.pictures.get(&row.id)? {
                Picture::Waiting(source) => Some((row.id, source.clone())),
                _ => None,
            })
    }
}

impl Inner {
    fn fill_pictures(self: &Rc<Self>, id: u64) {
        loop {
            let (visit, source, sizes, bytes, cancel) = {
                let mut switcher = self.switcher.borrow_mut();
                if switcher.captures >= pictures::MAX_IN_FLIGHT {
                    return;
                }
                let Some(opening) = switcher.showing(id).filter(|o| o.painted) else {
                    return;
                };
                let Some((visit, source)) = opening.next_picture() else {
                    return;
                };
                let allocation = source.allocation();
                let (bytes, scale) = allocation.unwrap_or((0, 1));
                if switcher.captures > 0
                    && switcher.bytes.saturating_add(bytes) > pictures::RAW_BUDGET
                {
                    return;
                }
                let sizes = Sizes {
                    large: opening
                        .capture_layout
                        .stage
                        .or(Some(opening.capture_layout.backdrop)),
                    thumb: (CARD_WIDTH, opening.capture_layout.picture_height),
                    scale,
                };
                let Phase::Showing(opening) = &mut switcher.phase else {
                    unreachable!();
                };
                if allocation.is_none() {
                    opening.pictures.insert(visit, Picture::Unavailable);
                    self.paint_picture(id, visit, None);
                    continue;
                }
                let cancel = gio::Cancellable::new();
                opening
                    .pictures
                    .insert(visit, Picture::InFlight(cancel.clone()));
                switcher.captures += 1;
                switcher.bytes += bytes;
                #[cfg(feature = "gui-smoke")]
                {
                    switcher.snapshots += 1;
                }
                (visit, source, sizes, bytes, cancel)
            };
            let weak = self.me.clone();
            let identity = source.clone();
            source.snapshot(&cancel, move |surface| {
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                let pixels = inner
                    .picture_in_flight(id, visit)
                    .then_some(surface)
                    .flatten()
                    .and_then(Pixels::copy);
                let Some(pixels) = pixels else {
                    inner.picture_finished(id, visit, bytes, None);
                    return;
                };
                let worker = gio::spawn_blocking(move || pixels.encode(sizes));
                glib::MainContext::default().spawn_local(async move {
                    let pair = worker.await.ok().flatten();
                    if let Some(inner) = weak.upgrade() {
                        inner.picture_finished(
                            id,
                            visit,
                            bytes,
                            pair.filter(|_| identity.retained()),
                        );
                    }
                });
            });
        }
    }

    fn picture_in_flight(&self, id: u64, visit: VisitId) -> bool {
        self.switcher.borrow().showing(id).is_some_and(|opening| {
            matches!(opening.pictures.get(&visit), Some(Picture::InFlight(_)))
        })
    }

    fn picture_finished(
        self: &Rc<Self>,
        id: u64,
        visit: VisitId,
        bytes: u64,
        pair: Option<pictures::Pair>,
    ) {
        let mut switcher = self.switcher.borrow_mut();
        switcher.captures -= 1;
        switcher.bytes -= bytes;
        let Some(opening) = (match &mut switcher.phase {
            Phase::Showing(opening) => Some(opening),
            _ => None,
        }) else {
            return;
        };
        let current = opening.id;
        if id == current && matches!(opening.pictures.get(&visit), Some(Picture::InFlight(_))) {
            self.paint_picture(id, visit, pair.as_ref());
            opening
                .pictures
                .insert(visit, pair.map_or(Picture::Unavailable, Picture::Ready));
        }
        drop(switcher);
        self.fill_pictures(current);
    }

    fn paint_picture(&self, opening: u64, visit: VisitId, pair: Option<&pictures::Pair>) {
        let path = format!("canopy://localhost/switcher-image/{opening}/{visit}");
        self.call(
            "switcher",
            &[
                json!("picture"),
                json!({
                    "opening": opening.to_string(),
                    "id": visit.to_string(),
                    "thumb": pair.map(|_| format!("{path}/thumb")),
                    "large": pair.filter(|pair| pair.has_large()).map(|_| format!("{path}/large")),
                }),
            ],
        );
    }
}

#[cfg(feature = "gui-smoke")]
impl HtmlChrome {
    pub fn switcher_snapshot_count(&self) -> u64 {
        self.inner.switcher.borrow().snapshots
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_dismiss_request_can_only_be_claimed_once() {
        let head = ulid::Ulid::new().to_string().parse().unwrap();
        let other = ulid::Ulid::new().to_string().parse().unwrap();
        let mut dismiss = Dismiss::Unclaimed(head);
        assert!(!dismiss.claim(other));
        assert!(dismiss.claim(head));
        assert!(!dismiss.claim(head));
    }

    #[test]
    fn full_roster_fits_page_area_and_scrolls_at_narrow_widths() {
        let here = ulid::Ulid::new().to_string().parse().unwrap();
        let mut rows = vec![Row {
            id: here,
            kind: Kind::Here,
        }];
        rows.extend((0..5).map(|_| Row {
            id: ulid::Ulid::new().to_string().parse().unwrap(),
            kind: Kind::Waiting,
        }));
        rows.extend((0..8).map(|_| Row {
            id: ulid::Ulid::new().to_string().parse().unwrap(),
            kind: Kind::Read,
        }));
        for (w, h) in [(720, 800), (1240, 800), (1920, 800), (1240, 300)] {
            let layout = FilmLayout::of((w.into(), h.into()), &rows, 3);
            let BoxPx(_, row_top, row_width, row_height) = layout.row;
            let BoxPx(_, caption_y, _, caption_h) = layout.caption;
            assert!(caption_y + caption_h <= row_top);
            assert!(row_top + row_height <= layout.footer.1);
            assert!(layout.footer.1 + layout.footer.3 <= h);
            assert_eq!(layout.cards.len(), 14);
            assert!(layout
                .cards
                .iter()
                .all(|card| card.1 .2 >= 112 && layout.picture_height >= 56));
            assert_eq!(layout.run.2 > row_width, w < 1920);
            let divider = layout.divider.unwrap();
            assert!(layout.cards[5].1 .0 + layout.cards[5].1 .2 < divider.0);
            assert!(divider.0 + divider.2 < layout.cards[6].1 .0);
            if h == 300 {
                assert!(layout.stage.is_none());
            } else {
                let BoxPx(x, y, sw, sh) = layout.stage.unwrap();
                let ratio = f64::from(w) / (f64::from(h) - CHROME_HEIGHT);
                assert!((f64::from(sw) / f64::from(sh) - ratio).abs() < 0.01);
                assert!(x >= 0 && x + sw <= w && y + sh <= caption_y);
            }
        }
    }
}
