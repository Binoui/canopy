use super::{items::Item, Level, Room};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Layout {
    #[default]
    Flow,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy)]
pub(super) struct Metrics {
    pub width: f64,
    pub height: f64,
    pub icon: f64,
    step: (f64, f64),
    gap: f64,
    label: f64,
}
impl Level {
    pub(super) fn metrics(self) -> Metrics {
        match self {
            Self::Far => Metrics {
                width: 76.,
                height: 60.,
                icon: 15.,
                step: (82., 64.),
                gap: 34.,
                label: 26.,
            },
            Self::Middle => Metrics {
                width: 132.,
                height: 106.,
                icon: 20.,
                step: (138., 110.),
                gap: 42.,
                label: 26.,
            },
            Self::Near => Metrics {
                width: 236.,
                height: 82.,
                icon: 18.,
                step: (258., 90.),
                gap: 46.,
                label: 26.,
            },
        }
    }
}
impl Metrics {
    pub fn css(self) -> Value {
        json!({"--item-w":self.width,"--item-h":self.height,"--label-w":124,"--card-w":236,"--card-h":46})
    }
}
struct Shape {
    at: Vec<Point>,
    width: f64,
    height: f64,
}
pub(super) struct Placed {
    pub trees: Vec<Point>,
    pub items: Vec<Vec<Point>>,
}
impl Layout {
    pub fn place(self, level: Level, trees: &[Vec<Item>], room: Room) -> Placed {
        let m = level.metrics();
        let shapes: Vec<_> = trees
            .iter()
            .map(|items| match self {
                Self::Flow => flow(items, m),
            })
            .collect();
        let band = level.band();
        let limit = (room.width - 32.) * band.min / band.rest;
        let (mut x, mut y, mut row_h) = (0., 0., 0_f64);
        let mut placed = Placed {
            trees: vec![],
            items: vec![],
        };
        for shape in shapes {
            if x > 0. && (level == Level::Near || x + shape.width > limit) {
                x = 0.;
                y += row_h + m.gap;
                row_h = 0.;
            }
            placed.trees.push(Point {
                x: x + m.width / 2.,
                y,
            });
            placed.items.push(
                shape
                    .at
                    .into_iter()
                    .map(|p| Point {
                        x: p.x + x,
                        y: p.y + y,
                    })
                    .collect(),
            );
            x += shape.width + m.gap;
            row_h = row_h.max(shape.height);
        }
        placed
    }
}
fn flow(items: &[Item], m: Metrics) -> Shape {
    let mut children = vec![Vec::new(); items.len()];
    let mut depth = vec![0; items.len()];
    for (i, item) in items.iter().enumerate() {
        if let Some(parent) = item.parent {
            children[parent].push(i);
            depth[i] = depth[parent] + 1;
        }
    }
    let mut at = vec![Point::default(); items.len()];
    let mut leaf = 0;
    for i in (0..items.len()).rev() {
        let y = if children[i].is_empty() {
            let y = leaf as f64 * m.step.1;
            leaf += 1;
            y
        } else {
            (at[children[i][0]].y + at[*children[i].last().unwrap()].y) / 2.
                - m.label
                - m.height / 2.
        };
        at[i] = Point {
            x: m.width / 2. + depth[i] as f64 * m.step.0,
            y: y + m.label + m.height / 2.,
        };
    }
    // Reverse postorder assigns leaves bottom to top; reflect to retain creation order.
    let height = (leaf.max(1) - 1) as f64 * m.step.1 + m.label + m.height;
    let bottom = at.iter().map(|p| p.y).fold(0_f64, f64::max);
    for p in &mut at {
        p.y = bottom - p.y + m.label + m.height / 2.;
    }
    Shape {
        width: depth
            .into_iter()
            .max()
            .map_or(m.width, |d| d as f64 * m.step.0 + m.width)
            .max(180.),
        height,
        at,
    }
}
pub(super) fn neighbours(points: &[Point]) -> Vec<[Option<usize>; 4]> {
    let mut out = vec![[None; 4]; points.len()];
    for axis in 0..2 {
        let coord = |p: Point| if axis == 0 { (p.x, p.y) } else { (p.y, p.x) };
        let mut order: Vec<_> = (0..points.len()).collect();
        order.sort_by(|&a, &b| {
            coord(points[a])
                .0
                .total_cmp(&coord(points[b]).0)
                .then(a.cmp(&b))
        });
        for &i in &order {
            let (a, b) = coord(points[i]);
            for forward in [false, true] {
                let mut best = f64::INFINITY;
                let boundary = order.partition_point(|&j| {
                    if forward {
                        coord(points[j]).0 <= a
                    } else {
                        coord(points[j]).0 < a
                    }
                });
                let mut offset = 0;
                loop {
                    let next = if forward {
                        boundary.checked_add(offset).filter(|n| *n < order.len())
                    } else {
                        boundary.checked_sub(offset + 1)
                    };
                    let Some(next) = next else { break };
                    offset += 1;
                    let j = order[next];
                    let (c, d) = coord(points[j]);
                    let along = (c - a).abs();
                    let across = (d - b).abs();
                    if along > best {
                        break;
                    }
                    let score = along + 2. * across;
                    if along > 0. && across <= along * 3_f64.sqrt() && score < best {
                        best = score;
                        out[i][axis * 2 + usize::from(forward)] = Some(j);
                    }
                }
            }
        }
    }
    out
}
pub(super) fn camera(
    placed: &Placed,
    home: Option<(usize, usize)>,
    room: Room,
    s: f64,
) -> [f64; 2] {
    let Some((t, i)) = home else {
        return [24., 32.];
    };
    let point = placed.items[t][i];
    let name = placed.trees[t];
    let mut pan = [
        room.width * 0.55 - point.x * s,
        room.height * 0.45 - point.y * s,
    ];
    if (point.x - name.x) * s < room.width - 180. {
        pan[0] = 112. - name.x * s;
    }
    if (point.y - name.y) * s < room.height - 100. {
        pan[1] = 32. - name.y * s;
    }
    pan
}
