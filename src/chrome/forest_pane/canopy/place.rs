use super::{items::Item, Level, Room};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Layout {
    #[default]
    Flow,
    Islands,
}
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy)]
pub(super) struct Metrics {
    pub icon: f64,
    step: f64,
    gap: f64,
    label: f64,
}
impl Level {
    pub(super) fn metrics(self) -> Metrics {
        match self {
            Self::Far => Metrics {
                icon: 15.,
                step: 58.,
                gap: 34.,
                label: 26.,
            },
            Self::Middle => Metrics {
                icon: 20.,
                step: 116.,
                gap: 42.,
                label: 26.,
            },
            Self::Near => Metrics {
                icon: 18.,
                step: 258.,
                gap: 46.,
                label: 26.,
            },
        }
    }
}
impl Metrics {
    pub fn css(self) -> Value {
        json!({"--label-w":124,"--card-w":236,"--card-h":46})
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
    bounds: Vec<(f64, f64)>,
}
impl Layout {
    pub fn name(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Islands => "islands",
        }
    }
    pub fn edge(self) -> &'static str {
        match self {
            Self::Flow => "curve",
            Self::Islands => "line",
        }
    }
    pub fn place(self, level: Level, trees: &[Vec<Item>], room: Room) -> Placed {
        let m = level.metrics();
        let shapes: Vec<_> = trees
            .iter()
            .map(|items| match self {
                Self::Flow => flow(items, m, level),
                Self::Islands => islands(items, m, level),
            })
            .collect();
        let limit = room.width - 48.;
        let mut occupied: Vec<(f64, f64, f64, f64)> = vec![];
        let mut placed = Placed {
            trees: vec![],
            items: vec![],
            bounds: vec![],
        };
        for shape in shapes {
            placed.bounds.push((shape.width, shape.height));
            let (x, y) = if self == Self::Flow && level == Level::Near {
                (
                    0.,
                    occupied.last().map_or(0., |&(_, y, _, h)| y + h + m.gap),
                )
            } else {
                let mut xs = vec![0.];
                let mut ys = vec![0.];
                for &(x, y, w, h) in &occupied {
                    xs.push(x + w + m.gap);
                    ys.push(y + h + m.gap);
                }
                ys.sort_by(f64::total_cmp);
                xs.sort_by(f64::total_cmp);
                ys.into_iter()
                    .find_map(|y| {
                        xs.iter()
                            .copied()
                            .find(|&x| {
                                (x == 0. || x + shape.width <= limit)
                                    && occupied.iter().all(|&(a, b, w, h)| {
                                        x + shape.width + m.gap <= a
                                            || a + w + m.gap <= x
                                            || y + shape.height + m.gap <= b
                                            || b + h + m.gap <= y
                                    })
                            })
                            .map(|x| (x, y))
                    })
                    .unwrap()
            };
            occupied.push((x, y, shape.width, shape.height));
            placed.trees.push(Point { x: x + 90., y });
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
        }
        placed
    }
}
fn flow(items: &[Item], m: Metrics, level: Level) -> Shape {
    let clearance = level.band().rest / level.band().min;
    let gap = if level == Level::Far { 4. } else { 8. };
    let mut children = vec![Vec::new(); items.len()];
    let mut depth = vec![0; items.len()];
    for (i, item) in items.iter().enumerate() {
        if let Some(parent) = item.parent {
            children[parent].push(i);
            depth[i] = depth[parent] + 1;
        }
    }
    let columns = depth.iter().max().map_or(0, |d| d + 1);
    let mut widths = vec![0_f64; columns];
    for (item, &d) in items.iter().zip(&depth) {
        widths[d] = widths[d].max(item.width(level));
    }
    let mut xs = vec![0.; columns];
    for d in 0..columns {
        xs[d] = if d == 0 {
            widths[d] / 2. * clearance
        } else {
            xs[d - 1]
                + m.step
                    .max(((widths[d - 1] + widths[d]) / 2. + gap) * clearance)
        };
    }
    let mut at = vec![Point::default(); items.len()];
    let mut contours: Vec<Vec<(f64, f64)>> = vec![vec![]; items.len()];
    for i in (0..items.len()).rev() {
        let mut contour: Vec<(f64, f64)> = vec![];
        for &child in &children[i] {
            let shift = contours[child]
                .iter()
                .zip(&contour)
                .map(|(&(top, _), &(_, bottom))| bottom + gap - top)
                .fold(0_f64, f64::max);
            let mut stack = vec![child];
            while let Some(node) = stack.pop() {
                at[node].y += shift;
                stack.extend(&children[node]);
            }
            for (d, &(top, bottom)) in contours[child].iter().enumerate() {
                if d == contour.len() {
                    contour.push((top + shift, bottom + shift));
                } else {
                    contour[d].0 = contour[d].0.min(top + shift);
                    contour[d].1 = contour[d].1.max(bottom + shift);
                }
            }
        }
        let y = children[i].first().map_or(0., |&first| {
            (at[first].y + at[*children[i].last().unwrap()].y) / 2.
        });
        at[i] = Point { x: xs[depth[i]], y };
        let (above, below) = items[i].extent(level);
        contour.insert(0, (y - above * clearance, y + below * clearance));
        contours[i] = contour;
    }
    let top = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.y - i.extent(level).0)
        .fold(0_f64, f64::min);
    let bottom = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.y + i.extent(level).1)
        .fold(0_f64, f64::max);
    for p in &mut at {
        p.y += m.label + 16. - top;
    }
    Shape {
        width: xs
            .last()
            .zip(widths.last())
            .map_or(180., |(x, w)| x + w / 2.)
            .max(180. * clearance),
        height: bottom - top + m.label + 16.,
        at,
    }
}
fn islands(items: &[Item], m: Metrics, level: Level) -> Shape {
    use std::f64::consts::PI;
    let clearance = level.band().rest / level.band().min;
    let diameter = items
        .iter()
        .map(|i| {
            let (above, below) = i.extent(level);
            i.width(level).hypot(2. * above.max(below)) + 20.
        })
        .fold(1_f64, f64::max)
        * clearance;
    let mut children = vec![vec![]; items.len()];
    let mut weight = vec![1; items.len()];
    for i in (1..items.len()).rev() {
        if let Some(p) = items[i].parent {
            weight[p] += weight[i];
        }
    }
    for (i, item) in items.iter().enumerate() {
        if let Some(p) = item.parent {
            children[p].push(i);
        }
    }
    let mut at = vec![Point::default(); items.len()];
    let mut start = -0.92 * PI;
    for &child in children.first().into_iter().flatten() {
        let span = 1.84 * PI * weight[child] as f64 / (items.len() - 1) as f64;
        let mut ring = (1. / (2. * (span / 2.).min(PI / 2.).sin())).ceil().max(1.) as usize;
        let mut used = 0;
        let mut stack = vec![child];
        while let Some(i) = stack.pop() {
            let (delta, capacity) = loop {
                let delta = 2. * (1. / (2. * ring as f64)).asin();
                let capacity = (span / delta).floor() as usize;
                if used < capacity {
                    break (delta, capacity);
                }
                ring += 1;
                used = 0;
            };
            let slot = if ring % 2 == 0 {
                capacity - used - 1
            } else {
                used
            };
            let angle = start + (span - capacity as f64 * delta) / 2. + (slot as f64 + 0.5) * delta;
            let radius = ring as f64 * diameter;
            at[i] = Point {
                x: radius * angle.cos(),
                y: radius * angle.sin(),
            };
            used += 1;
            stack.extend(children[i].iter().rev());
        }
        start += span;
    }
    let left = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.x - i.width(level) / 2. * clearance)
        .fold(0_f64, f64::min);
    let right = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.x + i.width(level) / 2. * clearance)
        .fold(180. * clearance, f64::max);
    let top = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.y - i.extent(level).0 * clearance)
        .fold(0_f64, f64::min);
    let bottom = at
        .iter()
        .zip(items)
        .map(|(p, i)| p.y + i.extent(level).1 * clearance)
        .fold(0_f64, f64::max);
    for p in &mut at {
        p.x -= left;
        p.y += m.label + 16. - top;
    }
    Shape {
        at,
        width: right - left,
        height: bottom - top + m.label + 16.,
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
    if placed.bounds[t].0 * s <= room.width - 48. {
        pan[0] = 112. - name.x * s;
    }
    if placed.bounds[t].1 * s <= room.height - 80. {
        pan[1] = 32. - name.y * s;
    }
    pan
}
