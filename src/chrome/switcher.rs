use crate::{browser::VisitId, forest::ForestProjection};

use super::waiting::Waiting;

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

use std::time::Duration;

use anyhow::Result;
use gtk::prelude::*;
use serde_json::{json, Value};
use wry::WebViewExtUnix;

use crate::pages::Pages;

use super::{view, ChromeFrame, ChromeIntent, HtmlChrome, Inner, CHROME_HEIGHT};

#[derive(Default)]
pub(super) struct Switcher {
    next: u64,
    phase: Phase,
}

#[derive(Default)]
enum Phase {
    #[default]
    Closed,
    Armed(Armed),
    Showing(Opening),
}

struct Armed {
    id: u64,
    choices: SwitchChoices,
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
    here: Option<VisitId>,
    selected: VisitId,
    rows: Vec<Row>,
    more_waiting: usize,
    focus_was_in_chrome: bool,
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

fn modifiers_held(window: &gtk::Window, modifiers: gdk::ModifierType) -> bool {
    gdk::Keymap::for_display(&window.display())
        .is_some_and(|keymap| keymap.modifier_state() & modifiers.bits() == modifiers.bits())
}

impl HtmlChrome {
    pub(crate) fn arm_switcher(
        &self,
        pages: &Pages,
        window: &gtk::Window,
        modifiers: gdk::ModifierType,
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
        if !modifiers_held(window, modifiers) {
            inner.open_switch_choice(first);
            return Ok(());
        }
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
                .is_some_and(|window| modifiers_held(&window, modifiers))
            {
                inner.reveal_switcher(None);
            } else {
                inner.finish_switcher(true);
            }
        });
        inner.switcher.borrow_mut().phase = Phase::Armed(Armed {
            id,
            choices,
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
        !matches!(self.inner.switcher.borrow().phase, Phase::Closed)
    }

    pub(crate) fn finish_switcher(&self, open: bool) {
        self.inner.finish_switcher(open);
    }
}

impl Inner {
    fn reveal_switcher(&self, step: Option<bool>) {
        let Phase::Armed(armed) = std::mem::take(&mut self.switcher.borrow_mut().phase) else {
            return;
        };
        let mut opening = Opening {
            id: armed.id,
            here: armed.choices.here,
            selected: armed.choices.first().unwrap(),
            rows: armed.choices.rows().collect(),
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
            let rows: Vec<_> = opening.rows.iter().map(|&row| row.value(frame)).collect();
            json!({
                "opening": opening.id.to_string(),
                "selected": opening.selected.to_string(),
                "rows": rows,
                "layout": FilmLayout::of(self.size.get(), &opening).value(),
                "moreWaiting": opening.more_waiting,
            })
        };
        self.switcher.borrow_mut().phase = Phase::Showing(opening);
        self.cover.set(None);
        self.apply_bounds();
        self.call("switcher", &[json!("show"), message]);
        let _ = self.view.focus();
    }

    pub(super) fn finish_switcher(&self, open: bool) {
        let phase = std::mem::take(&mut self.switcher.borrow_mut().phase);
        let choice = match phase {
            Phase::Closed => return,
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
                        "layout": FilmLayout::of(self.size.get(), opening).value(),
                    }),
                ],
            );
        }
    }

    pub(super) fn switcher_frame_changed(&self, current: Option<VisitId>) {
        let here = match &self.switcher.borrow().phase {
            Phase::Closed => return,
            Phase::Armed(armed) => armed.choices.here,
            Phase::Showing(opening) => opening.here,
        };
        if here != current {
            self.finish_switcher(false);
        }
    }

    pub(super) fn switcher_pointer(&self, message: &Value) {
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
        if message["opening"].as_str() != Some(opening.id.to_string().as_str())
            || !opening.rows.iter().any(|row| row.id == target)
        {
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
    cards: Vec<BoxPx>,
    divider: Option<BoxPx>,
    more: Option<BoxPx>,
    footer: BoxPx,
    picture_height: i32,
}

const CARD_WIDTH: i32 = 120;
const CARD_GAP: i32 = 10;
const ROW_INSET: i32 = 6;

impl FilmLayout {
    fn of((width, height): (f64, f64), opening: &Opening) -> Self {
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
        let has_waiting = opening.rows.iter().any(|row| row.kind == Kind::Waiting);
        let cards: Vec<_> = opening
            .rows
            .iter()
            .map(|row| {
                if row.kind == Kind::Read && has_waiting && divider.is_none() {
                    divider = Some(BoxPx(x, 6, 1, picture_height + 20));
                    x += CARD_GAP + 1;
                }
                let bounds = BoxPx(x, 6, CARD_WIDTH, picture_height + 22);
                x += CARD_WIDTH + CARD_GAP;
                bounds
            })
            .collect();
        let run_w = x - CARD_GAP + ROW_INSET;
        let more_w = if opening.more_waiting > 0 { 106 } else { 0 };
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
            "cards": self.cards.iter().copied().map(BoxPx::value).collect::<Vec<_>>(),
            "divider": self.divider.map(BoxPx::value),
            "more": self.more.map(BoxPx::value),
            "footer": self.footer.value(),
            "picture_height": self.picture_height,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let opening = Opening {
            id: 1,
            here: Some(here),
            selected: rows[1].id,
            rows,
            more_waiting: 3,
            focus_was_in_chrome: false,
        };
        for (w, h) in [(720, 800), (1240, 800), (1920, 800), (1240, 300)] {
            let layout = FilmLayout::of((w.into(), h.into()), &opening);
            let BoxPx(_, row_top, row_width, row_height) = layout.row;
            let BoxPx(_, caption_y, _, caption_h) = layout.caption;
            assert!(caption_y + caption_h <= row_top);
            assert!(row_top + row_height <= layout.footer.1);
            assert!(layout.footer.1 + layout.footer.3 <= h);
            assert_eq!(layout.cards.len(), 14);
            assert!(layout
                .cards
                .iter()
                .all(|card| card.2 >= 112 && layout.picture_height >= 56));
            assert_eq!(layout.run.2 > row_width, w < 1920);
            let divider = layout.divider.unwrap();
            assert!(layout.cards[5].0 + layout.cards[5].2 < divider.0);
            assert!(divider.0 + divider.2 < layout.cards[6].0);
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
