//! Pinned strip controls and the room left for the tree.

use super::{
    graph::{Rect, TRUNK_Y},
    waiting,
};
use crate::forest::ForestProjection;

const MARGIN: f64 = 8.;
const GAP: f64 = 5.;
const CLEARANCE: f64 = 10.;
const HEIGHT: f64 = 28.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pin {
    Waiting { count: usize },
}

impl Pin {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Waiting { .. } => "waiting",
        }
    }
    fn width(self) -> f64 {
        match self {
            Self::Waiting { count } => {
                7. + 16. + 7. + (count.to_string().len() as f64 * 6.5 + 10.) + 6.
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pins {
    pub room: f64,
    pub placed: Vec<(Pin, Rect)>,
}

pub(crate) fn pins(forest: &ForestProjection, width: f64) -> Pins {
    let count = waiting::pages(forest).len();
    place(
        (count > 0).then_some(Pin::Waiting { count }).into_iter(),
        width,
    )
}

fn place(shown: impl DoubleEndedIterator<Item = Pin>, width: f64) -> Pins {
    let mut right = width - MARGIN;
    let mut placed = Vec::new();
    for pin in shown.rev() {
        let w = pin.width();
        placed.push((
            pin,
            Rect {
                x: right - w / 2.,
                y: TRUNK_Y,
                w,
                s: HEIGHT,
            },
        ));
        right -= w + GAP;
    }
    placed.reverse();
    let room = placed
        .first()
        .map_or(width, |(_, r)| (r.x - r.w / 2. - CLEARANCE).max(0.));
    Pins { room, placed }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pin_geometry_reserves_only_visible_controls_and_grows_with_count() {
        assert_eq!(
            place(std::iter::empty(), 800.),
            Pins {
                room: 800.,
                placed: vec![]
            }
        );
        let mut previous = 0.;
        for count in [1, 12, 120] {
            let pins = place(std::iter::once(Pin::Waiting { count }), 800.);
            let r = pins.placed[0].1;
            assert!(r.w > previous);
            assert_eq!(r.x + r.w / 2., 792.);
            assert_eq!(pins.room + CLEARANCE, r.x - r.w / 2.);
            assert_eq!(r.y, TRUNK_Y);
            previous = r.w;
        }
        assert_eq!(
            place(std::iter::once(Pin::Waiting { count: 1 }), 20.).room,
            0.
        );
    }
}
