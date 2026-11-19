use super::Inner;
use gtk::prelude::*;
use serde_json::json;
use std::{cell::Cell, rc::Rc};

pub(super) fn connect(inner: &Rc<Inner>) {
    inner
        .pinch
        .set_propagation_phase(gtk::PropagationPhase::Capture);
    let scale = Rc::new(Cell::new(1.));
    let weak = Rc::downgrade(inner);
    let begin_scale = scale.clone();
    inner.pinch.connect_begin(move |gesture, _| {
        begin_scale.set(1.);
        let active = weak
            .upgrade()
            .is_some_and(|i| i.forest.borrow().shows_canopy());
        gesture.set_state(if active {
            gtk::EventSequenceState::Claimed
        } else {
            gtk::EventSequenceState::Denied
        });
    });
    let weak = Rc::downgrade(inner);
    inner.pinch.connect_scale_changed(move |gesture, next| {
        let Some(inner) = weak.upgrade() else { return };
        if !inner.forest.borrow().shows_canopy() {
            return;
        }
        if let Some(factor) = step(scale.get(), next) {
            scale.set(next);
            if let Some((x, y)) = gesture.bounding_box_center() {
                inner.call("canopyPinch", &[json!(factor), json!(x), json!(y)]);
            }
        }
    });
}
fn step(previous: f64, next: f64) -> Option<f64> {
    (previous.is_finite() && next.is_finite() && previous > 0. && next > 0.)
        .then_some(next / previous)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cumulative_gesture_scales_forward_only_the_increment() {
        assert_eq!(step(1., 1.25), Some(1.25));
        assert_eq!(step(1.25, 1.5), Some(1.2));
        assert_eq!(step(1.5, 0.75), Some(0.5));
        for n in [0., -1., f64::NAN, f64::INFINITY] {
            assert_eq!(step(1., n), None);
            assert_eq!(step(n, 1.), None);
        }
    }
}
