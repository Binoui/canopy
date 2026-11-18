mod items;
mod place;
use crate::chrome::ChromeFrame;
use items::Facts;
use place::{Layout, Point};
use serde_json::json;
#[derive(Clone, Copy)]
struct Room {
    width: f64,
    height: f64,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::chrome) enum Level {
    Far,
    Middle,
    Near,
}
struct Band {
    min: f64,
    max: f64,
    rest: f64,
}
impl Level {
    const ALL: [Self; 3] = [Self::Far, Self::Middle, Self::Near];
    fn name(self) -> &'static str {
        match self {
            Self::Far => "far",
            Self::Middle => "middle",
            Self::Near => "near",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Far => "Far",
            Self::Middle => "Middle",
            Self::Near => "Near",
        }
    }
    fn band(self) -> Band {
        match self {
            Self::Far => Band {
                min: 0.42,
                max: 0.8,
                rest: 0.62,
            },
            Self::Middle => Band {
                min: 0.8,
                max: 1.55,
                rest: 1.15,
            },
            Self::Near => Band {
                min: 1.55,
                max: 2.6,
                rest: 1.9,
            },
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == s)
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Far => "every open tree, one mark per site",
            Self::Middle => "sites folded, the pages that matter named",
            Self::Near => "every page by name",
        }
    }
}

#[cfg(test)]
mod tests;
