use super::*;
use crate::{address::WebOrigin, browser::PageScale};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageScaleAction {
    Increase,
    Decrease,
    Reset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PageScaleReadout {
    NoPage,
    Page { visit: VisitId, level: PageScale },
}

#[derive(Default)]
pub(in crate::pages) struct Views {
    views: RefCell<Vec<Weak<NativeState>>>,
}

impl Views {
    fn register(&self, state: &Rc<NativeState>) {
        self.views.borrow_mut().push(Rc::downgrade(state));
    }

    fn reapply_origin(&self, origin: &WebOrigin) -> Result<()> {
        let states: Vec<_> = {
            let mut views = self.views.borrow_mut();
            views.retain(|view| view.strong_count() > 0);
            views.iter().filter_map(Weak::upgrade).collect()
        };
        for state in states {
            if state.lifecycle.get() != Lifecycle::Released
                && state.page_origin().as_ref() == Some(origin)
            {
                state.apply_page_scale()?;
            }
        }
        Ok(())
    }
}

impl Pages {
    pub fn page_scale(&self, action: PageScaleAction) -> Result<()> {
        let origin = self
            .branches
            .live
            .borrow()
            .as_ref()
            .and_then(|page| page.state.page_origin());
        let Some(origin) = origin else { return Ok(()) };
        {
            let browser = self.browser.borrow();
            let level = browser.page_scale(&origin)?;
            let level = match action {
                PageScaleAction::Increase => level.larger(),
                PageScaleAction::Decrease => level.smaller(),
                PageScaleAction::Reset => PageScale::default(),
            };
            browser.set_page_scale(&origin, level)?;
        }
        self.scale_views.reapply_origin(&origin)?;
        self.publish_page_scale();
        Ok(())
    }

    pub(in crate::pages) fn publish_page_scale(&self) {
        let value = self
            .branches
            .live
            .borrow()
            .as_ref()
            .and_then(|page| {
                let visit = page.state.navigation.borrow().current.as_ref()?.visit;
                let level = PageScale::from_stored_percent(
                    (page.native.zoom_level() * 100.).round() as i64,
                )
                .unwrap_or_default();
                Some(PageScaleReadout::Page { visit, level })
            })
            .unwrap_or(PageScaleReadout::NoPage);
        self.receive(PageEvent::PageScale(value));
    }
}

impl NativeState {
    fn page_origin(&self) -> Option<WebOrigin> {
        self.navigation
            .borrow()
            .current
            .as_ref()
            .and_then(|document| WebOrigin::of(&document.page_url))
    }

    pub(super) fn apply_page_scale(&self) -> Result<()> {
        let level = self
            .page_origin()
            .map(|origin| self.owner().browser.borrow().page_scale(&origin))
            .transpose();
        let applied = match &level {
            Ok(Some(level)) => *level,
            _ => PageScale::default(),
        };
        let factor = f64::from(applied.percent()) / 100.;
        // A failed policy read must not carry the previous site's scale into this document.
        if (self.native.zoom_level() - factor).abs() > f64::EPSILON {
            self.native.set_zoom_level(factor);
        }
        level.map(|_| ())
    }
}

#[derive(Default)]
struct Wheel {
    document: u64,
    delta: f64,
}

impl Wheel {
    fn step(&mut self, document: u64, delta: f64) -> i32 {
        if self.document != document {
            self.delta = 0.;
            self.document = document;
        }
        self.delta += delta;
        let steps = self.delta.trunc() as i32;
        self.delta -= f64::from(steps);
        steps
    }
}

pub(super) fn setup(state: &Rc<NativeState>) -> Vec<(glib::Object, glib::SignalHandlerId)> {
    use webkit2gtk::SettingsExt;
    state.owner().scale_views.register(state);
    if let Some(settings) = WebViewExt::settings(&state.native) {
        settings.set_zoom_text_only(false);
    }
    state.native.add_events(
        gdk::EventMask::SCROLL_MASK
            | gdk::EventMask::SMOOTH_SCROLL_MASK
            | gdk::EventMask::KEY_RELEASE_MASK,
    );
    let wheel = Rc::new(RefCell::new(Wheel::default()));
    let weak = Rc::downgrade(state);
    let accumulation = wheel.clone();
    let scroll = state.native.connect_scroll_event(move |native, event| {
        let Some(state) = Pages::enter_native(&weak) else {
            return glib::Propagation::Proceed;
        };
        let owner = state.owner();
        let selected = owner
            .branches
            .live
            .borrow()
            .as_ref()
            .is_some_and(|page| page.native == *native);
        let modifiers = event.state() & gtk::accelerator_get_default_mod_mask();
        if !selected || !native.is_sensitive() || modifiers != gdk::ModifierType::CONTROL_MASK {
            accumulation.borrow_mut().delta = 0.;
            return glib::Propagation::Proceed;
        }
        let steps = match event.direction() {
            gdk::ScrollDirection::Up => {
                accumulation.borrow_mut().delta = 0.;
                -1
            }
            gdk::ScrollDirection::Down => {
                accumulation.borrow_mut().delta = 0.;
                1
            }
            gdk::ScrollDirection::Smooth => {
                let (dx, dy) = event.delta();
                if dy == 0. && dx != 0. {
                    return glib::Propagation::Proceed;
                }
                if !dy.is_finite() {
                    return glib::Propagation::Stop;
                }
                accumulation
                    .borrow_mut()
                    .step(state.history_document.get(), dy)
            }
            _ => return glib::Propagation::Proceed,
        };
        for _ in 0..steps.unsigned_abs() {
            state.report(owner.page_scale(if steps < 0 {
                PageScaleAction::Increase
            } else {
                PageScaleAction::Decrease
            }));
        }
        glib::Propagation::Stop
    });
    let document_wheel = wheel.clone();
    let document = state.native.connect_load_changed(move |_, event| {
        if event == webkit2gtk::LoadEvent::Committed {
            document_wheel.borrow_mut().delta = 0.;
        }
    });
    let keyboard = state
        .native
        .toplevel()
        .unwrap_or_else(|| state.native.clone().upcast());
    let release = keyboard.connect_key_release_event(move |_, event| {
        use gdk::keys::constants as key;
        if matches!(event.keyval(), key::Control_L | key::Control_R) {
            wheel.borrow_mut().delta = 0.;
        }
        glib::Propagation::Proceed
    });
    vec![
        (state.native.clone().upcast(), scroll),
        (keyboard.upcast(), release),
        (state.native.clone().upcast(), document),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn smooth_wheel_accumulates_and_document_changes_discard_remainder() {
        let mut wheel = Wheel::default();
        assert_eq!(wheel.step(1, -0.6), 0);
        assert_eq!(wheel.step(1, -0.6), -1);
        assert_eq!(wheel.step(1, 0.6), 0);
        assert_eq!(wheel.step(2, 0.7), 0);
        assert_eq!(wheel.step(2, 0.7), 1);
        assert_eq!(wheel.step(2, 2.), 2);
    }
}
