use super::{items::Item, Level, Room};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Layout {
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
    layout: Layout,
    pub trees: Vec<Point>,
    pub items: Vec<Vec<Point>>,
    bounds: Vec<(f64, f64)>,
}
impl Layout {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flow" => Some(Self::Flow),
            "islands" => Some(Self::Islands),
            _ => None,
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Flow => "Flow",
            Self::Islands => "Islands",
        }
    }
    pub(super) fn choices() -> Value {
        json!([Self::Flow, Self::Islands].map(|l| json!({"layout":l.name(),"label":l.label()})))
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Flow => "flow",
            Self::Islands => "islands",
        }
    }
    pub(super) fn edge(self) -> &'static str {
        match self {
            Self::Flow => "curve",
            Self::Islands => "line",
        }
    }
    pub(super) fn place(self, level: Level, trees: &[Vec<Item>], room: Room) -> Placed {
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
            layout: self,
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
#[derive(Clone, Copy)]
pub(super) struct Sector {
    pub start: f64,
    pub end: f64,
}
impl Sector {
    fn middle(self) -> f64 {
        (self.start + self.end) / 2.
    }
}
#[derive(Clone, Copy)]
struct BoxSize {
    half_width: f64,
    above: f64,
    below: f64,
}
impl BoxSize {
    fn overlaps(self, a: Point, other: Self, b: Point) -> bool {
        (a.x - b.x).abs() < self.half_width + other.half_width
            && a.y - self.above < b.y + other.below
            && b.y - other.above < a.y + self.below
    }
    fn crosses(self, center: Point, a: Point, b: Point) -> bool {
        let mut lo = 0_f64;
        let mut hi = 1_f64;
        for (origin, delta, min, max) in [
            (
                a.x,
                b.x - a.x,
                center.x - self.half_width,
                center.x + self.half_width,
            ),
            (a.y, b.y - a.y, center.y - self.above, center.y + self.below),
        ] {
            if delta.abs() < 1e-9 {
                if origin < min || origin > max {
                    return false;
                }
            } else {
                let u = (min - origin) / delta;
                let v = (max - origin) / delta;
                lo = lo.max(u.min(v));
                hi = hi.min(u.max(v));
            }
        }
        lo <= hi
    }
}
fn fit_sectors(
    at: &mut [Point],
    sectors: &mut [Sector],
    boxes: &[BoxSize],
    items: &[Item],
    clearance: f64,
    minimum: f64,
) {
    use std::f64::consts::PI;
    let mut best = (f64::INFINITY, 1., 1., 0.);
    for phase in 0..24 {
        let rotation = phase as f64 * PI / 12.;
        let rotated: Vec<_> = at
            .iter()
            .map(|p| Point {
                x: p.x * rotation.cos() - p.y * rotation.sin(),
                y: p.x * rotation.sin() + p.y * rotation.cos(),
            })
            .collect();
        let at = &rotated;
        for trial in 0..80 {
            let sy = minimum * 1.06_f64.powi(trial);
            let mut sx = minimum;
            for i in 0..items.len() {
                for j in 0..i {
                    if at[i].y * sy - boxes[i].above < at[j].y * sy + boxes[j].below
                        && at[j].y * sy - boxes[j].above < at[i].y * sy + boxes[i].below
                    {
                        sx = sx.max(
                            (boxes[i].half_width + boxes[j].half_width) / (at[i].x - at[j].x).abs(),
                        );
                    }
                }
                if let Some(parent) = items[i].parent {
                    for j in 0..items.len() {
                        if j == i || j == parent {
                            continue;
                        }
                        let a = at[parent];
                        let b = at[i];
                        let c = at[j];
                        let dy = (b.y - a.y) * sy;
                        let (lo, hi) = if dy.abs() < 1e-9 {
                            if a.y * sy < c.y * sy - boxes[j].above
                                || a.y * sy > c.y * sy + boxes[j].below
                            {
                                continue;
                            }
                            (0., 1.)
                        } else {
                            let u = ((c.y - a.y) * sy - boxes[j].above) / dy;
                            let v = ((c.y - a.y) * sy + boxes[j].below) / dy;
                            (u.min(v).max(0.), u.max(v).min(1.))
                        };
                        if lo > hi {
                            continue;
                        }
                        let x0 = a.x + (b.x - a.x) * lo - c.x;
                        let x1 = a.x + (b.x - a.x) * hi - c.x;
                        let distance = if x0 * x1 <= 0. {
                            0.
                        } else {
                            x0.abs().min(x1.abs())
                        };
                        sx = sx.max(boxes[j].half_width / distance);
                    }
                }
            }
            let left = at
                .iter()
                .zip(boxes)
                .map(|(p, b)| p.x * sx - b.half_width)
                .fold(0_f64, f64::min);
            let right = at
                .iter()
                .zip(boxes)
                .map(|(p, b)| p.x * sx + b.half_width)
                .fold(180. * clearance, f64::max);
            let top = at
                .iter()
                .zip(boxes)
                .map(|(p, b)| p.y * sy - b.above)
                .fold(0_f64, f64::min);
            let bottom = at
                .iter()
                .zip(boxes)
                .map(|(p, b)| p.y * sy + b.below)
                .fold(0_f64, f64::max);
            let score = ((right - left) / 1384.).max((bottom - top + 42.) / 744.);
            if score < best.0 {
                best = (score, sx, sy, rotation);
            }
        }
    }
    for p in &mut *at {
        let x = p.x * best.3.cos() - p.y * best.3.sin();
        let y = p.x * best.3.sin() + p.y * best.3.cos();
        p.x = x * best.1 * 1.001;
        p.y = y * best.2 * 1.001;
    }
    let transform = |angle: f64| {
        let angle = angle + best.3;
        let mut a = (angle.sin() * best.2).atan2(angle.cos() * best.1);
        while a < angle - PI {
            a += 2. * PI;
        }
        while a > angle + PI {
            a -= 2. * PI;
        }
        a
    };
    for sector in &mut *sectors {
        sector.start = transform(sector.start);
        sector.end = transform(sector.end);
    }
}
pub(super) fn island_points(items: &[Item], level: Level) -> (Vec<Point>, Vec<Sector>) {
    use std::f64::consts::PI;
    let clearance = level.band().rest / level.band().min;
    let boxes: Vec<_> = items
        .iter()
        .map(|item| {
            let (above, below) = item.extent(level);
            BoxSize {
                half_width: item.width(level) / 2. * clearance,
                above: above * clearance,
                below: below * clearance,
            }
        })
        .collect();
    let mut children = vec![Vec::new(); items.len()];
    for (i, item) in items.iter().enumerate() {
        if let Some(parent) = item.parent {
            children[parent].push(i);
        }
    }
    let mut weight: Vec<_> = boxes.iter().map(|b| b.half_width).collect();
    let mut tail = vec![1; items.len()];
    for i in (0..items.len()).rev() {
        if !children[i].is_empty() {
            weight[i] = children[i]
                .iter()
                .map(|&c| weight[c])
                .sum::<f64>()
                .max(weight[i]);
            if children[i].len() == 1 {
                tail[i] += tail[children[i][0]];
            }
        }
    }
    let step = match level {
        Level::Far => 54.,
        Level::Middle => 104.,
        Level::Near => 244.,
    };
    let whole = Sector {
        start: -PI / 2.,
        end: PI * 1.5,
    };
    let mut sectors = vec![whole; items.len()];
    let mut radii = vec![0.; items.len()];
    let mut at = vec![Point::default(); items.len()];
    for i in 0..items.len() {
        let sector = sectors[i];
        let angle = sector.middle();
        if i > 0 {
            at[i] = Point {
                x: radii[i] * angle.cos(),
                y: radii[i] * angle.sin(),
            };
        }
        let mut start = sector.start;
        let span = if i == 0 || children[i].len() == 1 {
            sector.end - sector.start
        } else {
            (sector.end - sector.start).min(PI * 2. / 3.)
        };
        start += (sector.end - sector.start - span) / 2.;
        let total: f64 = children[i].iter().map(|&c| weight[c]).sum();
        for &child in &children[i] {
            let share = span * weight[child] / total;
            sectors[child] = Sector {
                start,
                end: start + share,
            };
            radii[child] = radii[i] + step;
            start += share;
        }
    }
    for head in 0..items.len() {
        if tail[head] < 7 || items[head].parent.is_some_and(|p| children[p].len() == 1) {
            continue;
        }
        let mut run = vec![head];
        while children[*run.last().unwrap()].len() == 1 {
            run.push(children[*run.last().unwrap()][0]);
        }
        let sector = sectors[head];
        let mut angle = sector.middle();
        let mut radius = radii[run[5]];
        let pitch = run
            .iter()
            .map(|&i| boxes[i].above + boxes[i].below)
            .fold(0_f64, f64::max);
        let delta = 0.025_f64.min((sector.end - sector.start) / 16.);
        let mut forward = true;
        for (offset, &i) in run.iter().enumerate().skip(6) {
            loop {
                angle += if forward { delta } else { -delta };
                if sector.end - sector.start > 2. * PI - 0.01 {
                    radius += pitch * delta / (2. * PI);
                    if angle > sector.end {
                        angle -= 2. * PI;
                    }
                } else if angle >= sector.end - delta || angle <= sector.start + delta {
                    angle = angle.clamp(sector.start + delta, sector.end - delta);
                    radius += pitch;
                    forward = !forward;
                }
                let p = Point {
                    x: radius * angle.cos(),
                    y: radius * angle.sin(),
                };
                if run[..offset].iter().all(|&j| {
                    !boxes[i].overlaps(p, boxes[j], at[j])
                        && (j == run[offset - 1]
                            || !boxes[j].crosses(at[j], at[run[offset - 1]], p))
                }) && run[..offset]
                    .windows(2)
                    .all(|edge| !boxes[i].crosses(p, at[edge[0]], at[edge[1]]))
                {
                    at[i] = p;
                    break;
                }
            }
        }
    }
    for i in 0..items.len() {
        if children[i].len() <= 32 || children[i].iter().any(|&c| !children[c].is_empty()) {
            continue;
        }
        let sector = sectors[i];
        let delta = 0.025_f64.min((sector.end - sector.start) / 16.);
        let mut angle = sector.start;
        let mut radius = radii[i] + step;
        let pitch = children[i]
            .iter()
            .map(|&c| boxes[c].above + boxes[c].below)
            .fold(0_f64, f64::max);
        for &child in &children[i] {
            loop {
                angle += delta;
                if angle > sector.end - delta {
                    angle = sector.start + delta;
                    radius += pitch;
                }
                let p = Point {
                    x: radius * angle.cos(),
                    y: radius * angle.sin(),
                };
                if (0..child).all(|j| !boxes[child].overlaps(p, boxes[j], at[j])) {
                    at[child] = p;
                    sectors[child] = Sector {
                        start: angle - delta / 2.,
                        end: angle + delta / 2.,
                    };
                    break;
                }
            }
        }
    }
    let leaf_rows = children
        .iter()
        .any(|c| c.len() > 32 && c.iter().all(|&i| children[i].is_empty()));
    if items.len() > 3 && tail.iter().all(|&n| n < 7) && !leaf_rows {
        fit_sectors(
            &mut at,
            &mut sectors,
            &boxes,
            items,
            clearance,
            if level == Level::Middle { 0.4 } else { 0.9 },
        );
        return (at, sectors);
    }
    let mut scale = 1_f64;
    for i in 0..items.len() {
        for j in 0..i {
            let dx = (at[i].x - at[j].x).abs();
            let dy = at[i].y - at[j].y;
            let x = (boxes[i].half_width + boxes[j].half_width) / dx;
            let y = if dy > 0. {
                (boxes[j].below + boxes[i].above) / dy
            } else {
                (boxes[i].below + boxes[j].above) / -dy
            };
            scale = scale.max(x.min(y));
        }
    }
    for p in &mut at {
        p.x *= scale;
        p.y *= scale;
    }
    if !leaf_rows {
        while items.iter().enumerate().any(|(i, item)| {
            item.parent.is_some_and(|parent| {
                (0..items.len())
                    .any(|j| j != i && j != parent && boxes[j].crosses(at[j], at[parent], at[i]))
            })
        }) {
            for p in &mut at {
                p.x *= 1.05;
                p.y *= 1.05;
            }
        }
    }
    (at, sectors)
}
fn islands(items: &[Item], m: Metrics, level: Level) -> Shape {
    let clearance = level.band().rest / level.band().min;
    let (mut at, _) = island_points(items, level);
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
    if placed.layout == Layout::Islands {
        if placed.bounds[t].0 * s <= room.width - 16. {
            pan[0] = (room.width - placed.bounds[t].0 * s) / 2. - (name.x - 90.) * s;
        }
        if placed.bounds[t].1 * s <= room.height - 16. {
            pan[1] = (room.height - placed.bounds[t].1 * s) / 2. - name.y * s;
        }
        return pan;
    }
    if placed.bounds[t].0 * s <= room.width - 48. {
        pan[0] = 112. - name.x * s;
    }
    if placed.bounds[t].1 * s <= room.height - 80. {
        pan[1] = 32. - name.y * s;
    }
    pan
}
