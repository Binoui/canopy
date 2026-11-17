//! Pinned strip controls and the room left for the tree.

use super::{
    graph::{Rect, TRUNK_Y},
    waiting,
};
use crate::{forest::ForestProjection, mail::Mail};

const MARGIN: f64 = 8.;
const GAP: f64 = 5.;
const CLEARANCE: f64 = 10.;
const HEIGHT: f64 = 28.;
/// Back and Forward sit at the left end of the strip, before the tree: an 8 px margin,
/// two 28 px buttons and a 2 px gap between them.
pub(crate) const NAV: f64 = 66.;
/// The least room the tree keeps beside Back and Forward. Narrower, the buttons give way.
pub(crate) const NAV_MIN_ROOM: f64 = 280.;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Pin {
    Waiting { count: usize },
    Mail { unread: u32 },
    Zoom,
}

impl Pin {
    pub(crate) fn kind(self) -> &'static str {
        match self {
            Self::Waiting { .. } => "waiting",
            Self::Zoom => "zoom",
            Self::Mail { .. } => "mail",
        }
    }
    fn width(self) -> f64 {
        match self {
            Self::Zoom | Self::Mail { unread: 0 } => HEIGHT,
            Self::Mail { unread } => counted(unread as usize),
            Self::Waiting { count } => counted(count),
        }
    }
}

fn counted(count: usize) -> f64 {
    7. + 16. + 7. + (count.to_string().len() as f64 * 6.5 + 10.) + 6.
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Pins {
    pub room: f64,
    pub placed: Vec<(Pin, Rect)>,
}

pub(crate) fn pins(forest: &ForestProjection, mail: &Mail, width: f64) -> Pins {
    let count = waiting::pages(forest).len();
    place(
        (count > 0)
            .then_some(Pin::Waiting { count })
            .into_iter()
            .chain((!mail.accounts.is_empty()).then(|| Pin::Mail {
                unread: mail.unread(),
            }))
            .chain(forest.current().map(|_| Pin::Zoom)),
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
    fn zoom_is_rightmost_and_the_controls_do_not_overlap() {
        let pins = place([Pin::Waiting { count: 3 }, Pin::Zoom].into_iter(), 800.);
        let a = pins.placed[0].1;
        let b = pins.placed[1].1;
        assert!(a.x + a.w / 2. < b.x - b.w / 2.);
        assert_eq!(b.x + b.w / 2., 792.);
        assert_eq!(pins.room + CLEARANCE, a.x - a.w / 2.);
    }
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

#[cfg(test)]
mod mail_tests {
    use super::*;

    #[test]
    fn mail_sits_between_waiting_and_zoom_and_grows_with_its_digits() {
        let pins = place(
            [
                Pin::Waiting { count: 2 },
                Pin::Mail { unread: 3 },
                Pin::Zoom,
            ]
            .into_iter(),
            800.,
        );
        assert_eq!(
            pins.placed
                .iter()
                .map(|(p, _)| p.kind())
                .collect::<Vec<_>>(),
            ["waiting", "mail", "zoom"]
        );
        assert_eq!(pins.placed[2].1.x + pins.placed[2].1.w / 2., 792.);
        assert!(pins
            .placed
            .windows(2)
            .all(|p| p[0].1.x + p[0].1.w / 2. < p[1].1.x - p[1].1.w / 2.));
        assert_eq!(Pin::Mail { unread: 0 }.width(), HEIGHT);
        assert!(Pin::Mail { unread: 12 }.width() > Pin::Mail { unread: 1 }.width());
        assert!(Pin::Mail { unread: 123 }.width() > Pin::Mail { unread: 12 }.width());
    }
}
