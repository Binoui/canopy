use std::num::NonZeroUsize;

use super::waiting::Waiting;
use crate::{
    browser::{VisitId, VisitState},
    forest::ForestProjection,
};

pub(crate) const REST_HEIGHT: f64 = 52.;
pub(crate) const TRUNK_Y: f64 = REST_HEIGHT / 2.;
const GAP: f64 = 22.;
const PAD: f64 = 14.;
pub(crate) const ITEM_LEFT: f64 = 6.;
pub(crate) const ITEM_RIGHT: f64 = 9.;
pub(crate) const ITEM_BORDER: f64 = 1.;
pub(crate) const ITEM_ICON: f64 = 14.;
pub(crate) const PARENT_GAP: f64 = 6.;
pub(crate) const RUN_GAP: f64 = 5.;
const PARENT_MAX: f64 = 240.;
const PATH_ITEMS: usize = 3;

fn label_width(label: &str, gap: f64, size: f64) -> f64 {
    ITEM_LEFT
        + ITEM_RIGHT
        + 2. * ITEM_BORDER
        + ITEM_ICON
        + gap
        + super::view::text_width(label, size, 0.65).ceil()
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct Rect {
    pub x: f64,
    pub y: f64,
    pub s: f64,
    pub w: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum EdgeKind {
    Path,
    Pending,
    Plain,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Edge {
    pub from: VisitId,
    pub to: VisitId,
    pub kind: EdgeKind,
    pub from_bottom: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StripItem {
    Root(VisitId),
    Page(VisitId),
    Parent(VisitId),
    Run {
        ids: Vec<VisitId>,
        host: String,
    },
    Earlier {
        ids: Vec<VisitId>,
    },
    Current(VisitId),
    After {
        read: Vec<VisitId>,
    },
    Tab {
        id: VisitId,
        below: bool,
        named: bool,
        divider: bool,
    },
    Overflow {
        hidden: NonZeroUsize,
    },
}

impl StripItem {
    pub fn ids(&self) -> &[VisitId] {
        match self {
            Self::Root(id) | Self::Page(id) | Self::Parent(id) | Self::Current(id) => {
                std::slice::from_ref(id)
            }
            Self::Run { ids, .. } | Self::Earlier { ids } => ids,
            Self::Tab { id, .. } => std::slice::from_ref(id),
            Self::After { .. } | Self::Overflow { .. } => &[],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct BranchCounts {
    pub pages: usize,
    pub waiting: usize,
    pub read: usize,
}

pub(crate) struct StripNode {
    pub item: StripItem,
    pub rect: Rect,
    pub branches: BranchCounts,
    pub rows: Vec<VisitId>,
    pub total_rows: usize,
}

pub(crate) fn branch_heads(
    forest: &ForestProjection,
    ids: &[VisitId],
    path: &[VisitId],
) -> Vec<VisitId> {
    let path: std::collections::HashSet<_> = path.iter().copied().collect();
    let mut heads: Vec<_> = ids
        .iter()
        .flat_map(|id| forest.children(*id))
        .copied()
        .filter(|id| !path.contains(id))
        .collect();
    heads.sort_by_key(|id| {
        std::cmp::Reverse((forest.visit(*id).unwrap().created_at, id.to_string()))
    });
    heads
}

pub(crate) fn branch_counts(forest: &ForestProjection, heads: &[VisitId]) -> BranchCounts {
    let mut counts = BranchCounts::default();
    let mut stack = heads.to_vec();
    while let Some(id) = stack.pop() {
        if forest.is_settled(id) {
            continue;
        }
        counts.pages += 1;
        counts.waiting += usize::from(forest.is_waiting(id));
        counts.read += usize::from(forest.visit(id).unwrap().state == VisitState::Opened);
        stack.extend(forest.children(id));
    }
    counts
}

pub(crate) fn strip_items(forest: &ForestProjection, current: VisitId) -> Vec<StripItem> {
    let path = forest.path(current);
    let mut items = Vec::new();
    if path.len() > 1 {
        items.push(StripItem::Root(path[0]));
    }
    let mut i = 1;
    while i + 2 < path.len() {
        let start = i;
        let host = forest
            .visit(path[i])
            .unwrap()
            .url
            .host_str()
            .unwrap_or_default();
        i += 1;
        while i + 2 < path.len() && forest.visit(path[i]).unwrap().url.host_str() == Some(host) {
            i += 1;
        }
        items.push(if i - start > 1 {
            StripItem::Run {
                ids: path[start..i].to_vec(),
                host: host.into(),
            }
        } else {
            StripItem::Page(path[start])
        });
    }
    if path.len() > 2 {
        items.push(StripItem::Parent(path[path.len() - 2]));
    }
    items.push(StripItem::Current(current));
    let mut read: Vec<_> = forest
        .children(current)
        .iter()
        .copied()
        .filter(|id| {
            !forest.is_settled(*id) && forest.visit(*id).unwrap().state == VisitState::Opened
        })
        .collect();
    read.sort_by_key(|id| {
        std::cmp::Reverse((forest.visit(*id).unwrap().created_at, id.to_string()))
    });
    if !read.is_empty() {
        items.push(StripItem::After { read });
    }
    items
}

pub(crate) fn short_name(forest: &ForestProjection, id: VisitId) -> String {
    let v = forest.visit(id).unwrap();
    let name = crate::naming::page_name(&v.title, &v.url);
    v.parent.and_then(|id| forest.visit(id)).map_or_else(
        || name.clone(),
        |parent| distinct_name(&name, &crate::naming::page_name(&parent.title, &parent.url)),
    )
}

pub(crate) fn distinct_name(name: &str, related: &str) -> String {
    for sep in [" · ", " - ", " | ", " — ", " – ", " : "] {
        let pieces: Vec<_> = name.split(sep).collect();
        let parent_pieces: Vec<_> = related.split(sep).collect();
        let unique: Vec<_> = pieces
            .iter()
            .filter(|p| !parent_pieces.contains(p))
            .copied()
            .collect();
        if !unique.is_empty() && unique.len() < pieces.len() {
            return unique.join(sep);
        }
    }
    name.to_owned()
}

pub(crate) fn parent_name(forest: &ForestProjection, id: VisitId, current: VisitId) -> String {
    let name = short_name(forest, id);
    let v = forest.visit(current).unwrap();
    let related = crate::naming::page_name(&v.title, &v.url);
    let name = distinct_name(&name, &related);
    let parent = forest.visit(id).unwrap();
    let related = distinct_name(
        &related,
        &crate::naming::page_name(&parent.title, &parent.url),
    );
    if name == related {
        return name;
    }
    let common_end = name
        .char_indices()
        .zip(related.chars())
        .take_while(|((_, a), b)| *a == *b)
        .filter(|((_, c), _)| c.is_whitespace())
        .map(|((i, c), _)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    let suffix = name[common_end..].trim();
    if suffix.is_empty() {
        name
    } else {
        suffix.to_owned()
    }
}

fn item_width(forest: &ForestProjection, item: &StripItem, pill: f64, current: VisitId) -> f64 {
    match item {
        StripItem::Root(id) if forest.visit(current).unwrap().parent == Some(*id) => {
            label_width(&parent_name(forest, *id, current), PARENT_GAP, 12.5).min(PARENT_MAX)
        }
        StripItem::Root(_) | StripItem::Page(_) => 28.,
        StripItem::Parent(id) => {
            label_width(&parent_name(forest, *id, current), PARENT_GAP, 12.5).min(PARENT_MAX)
        }
        StripItem::Run { ids, .. } => label_width(&ids.len().to_string(), RUN_GAP, 11.5),
        StripItem::Earlier { ids } => 76. + ids.len().to_string().len() as f64 * 7.,
        StripItem::Current(_) => pill,
        StripItem::After { read } => 22. + read.len().to_string().len() as f64 * 7.,
        StripItem::Tab { .. } | StripItem::Overflow { .. } => {
            unreachable!("shelf items have their own widths")
        }
    }
}

const TAB: f64 = 28.;
const TAB_MAX: f64 = 158.;
const TAB_GAP: f64 = 5.;
pub(crate) const DIVIDER_GAP: f64 = 13.;
const PARENT_FLOOR: f64 = 60.;
const PILL_FLOOR: f64 = 196.;
const NARROW_PILL: f64 = 52.;

#[derive(Clone, Copy)]
struct Flex {
    natural: f64,
    floor: f64,
}

impl Flex {
    fn new(natural: f64, floor: f64) -> Self {
        Self {
            natural,
            floor: natural.min(floor),
        }
    }
}

struct ShelfRoom<'a> {
    free: f64,
    parent: Flex,
    pill: Flex,
    names: &'a [f64],
    total: usize,
    below: usize,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shelf {
    All { named: usize },
    Cut { shown: usize, hidden: NonZeroUsize },
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct ShelfFit {
    shelf: Shelf,
    parent: f64,
    pill: f64,
}

fn fit_shelf(room: &ShelfRoom<'_>) -> ShelfFit {
    let done = |shelf, width| {
        let (parent, pill) = share(room.free - width, room.parent, room.pill, room.total > 0);
        ShelfFit {
            shelf,
            parent,
            pill,
        }
    };
    if room.total == 0 {
        return done(Shelf::All { named: 0 }, 0.);
    }
    let mut extras = vec![0.];
    for name in room.names {
        extras.push(extras.last().unwrap() + name - TAB);
    }
    let floors = room.parent.floor + room.pill.floor;
    let icons = shelf_width(room.total, room.below, room.total);
    for named in
        std::iter::once(room.total).chain([3, 2, 1, 0].into_iter().filter(|n| *n < room.total))
    {
        if let Some(extra) = extras.get(named) {
            if icons + extra + floors <= room.free {
                // Past the rung, a name is added only where it costs the pill and the
                // parent's name nothing.
                let spare = room.free - room.parent.natural - room.pill.natural - icons;
                let named = (named..extras.len())
                    .rev()
                    .find(|more| extras[*more] <= spare)
                    .unwrap_or(named);
                return done(Shelf::All { named }, icons + extras[named]);
            }
        }
    }
    if room.total == 1 {
        return done(Shelf::All { named: 0 }, least_shelf(1));
    }
    let shown = (2..measured(room.free, room.total))
        .rev()
        .find(|shown| shelf_width(*shown, room.below, room.total) + floors <= room.free)
        .unwrap_or(1);
    let hidden = NonZeroUsize::new(room.total - shown).unwrap();
    done(
        Shelf::Cut { shown, hidden },
        shelf_width(shown, room.below, room.total),
    )
}

fn least_shelf(total: usize) -> f64 {
    shelf_width(total.min(1), 0, total)
}

fn measured(free: f64, total: usize) -> usize {
    total.min((free.max(0.) / (TAB + TAB_GAP)).floor() as usize + 1)
}

fn divider_at(below: usize, shown: usize) -> Option<usize> {
    (below > 0 && below < shown).then_some(below)
}

fn shelf_width(shown: usize, below: usize, total: usize) -> f64 {
    if shown == 0 {
        return 0.;
    }
    let icons = GAP + shown as f64 * TAB + (shown - 1) as f64 * TAB_GAP;
    let divider = if divider_at(below, shown).is_some() {
        DIVIDER_GAP - TAB_GAP
    } else {
        0.
    };
    let overflow = if shown < total {
        TAB_GAP + overflow_width(total - shown)
    } else {
        0.
    };
    icons + divider + overflow
}

fn share(free: f64, parent: Flex, pill: Flex, waiting: bool) -> (f64, f64) {
    let free = free.max(0.);
    let keep = if waiting {
        pill.natural.min(NARROW_PILL)
    } else {
        0.
    };
    let parent_width = parent.natural.min(
        (free - pill.natural)
            .max(parent.floor)
            .min((free - keep).max(0.)),
    );
    (parent_width, pill.natural.min(free - parent_width))
}

fn tab_width(name: &str) -> f64 {
    (8. + 16. + 8. + super::view::text_width(name, 12.5, 0.56).ceil() + 20. + 2.).min(TAB_MAX)
}

fn overflow_width(hidden: usize) -> f64 {
    (16. + (1 + hidden.ilog10() + 1) as f64 * 7.).max(TAB)
}

pub(crate) struct StripLayout {
    pub trail: Vec<StripNode>,
    pub shelf: Vec<StripNode>,
}

pub(crate) fn strip_layout(
    forest: &ForestProjection,
    current: VisitId,
    room: f64,
    pill: f64,
    waiting: &Waiting<'_>,
) -> StripLayout {
    let mut items = strip_items(forest, current);
    let width = |items: &[StripItem]| -> f64 {
        let widths: Vec<_> = items
            .iter()
            .map(|item| item_width(forest, item, pill, current))
            .filter(|w| *w > 0.)
            .collect();
        PAD * 2. + widths.iter().sum::<f64>() + GAP * widths.len().saturating_sub(1) as f64
    };
    let middle_count = |items: &[StripItem]| {
        items
            .iter()
            .filter(|item| matches!(item, StripItem::Page(_) | StripItem::Run { .. }))
            .count()
    };
    let parent_saving = |items: &[StripItem]| {
        items
            .iter()
            .filter(|item| matches!(item, StripItem::Parent(_)))
            .map(|item| (item_width(forest, item, pill, current) - PARENT_FLOOR).max(0.))
            .sum::<f64>()
    };
    let least = least_shelf(waiting.len());
    while (middle_count(&items) > PATH_ITEMS
        || width(&items) - parent_saving(&items) > room - least)
        && items.len() > 4
    {
        if !matches!(items[2], StripItem::Page(_) | StripItem::Run { .. }) {
            break;
        }
        if !matches!(items[1], StripItem::Earlier { .. }) {
            if !matches!(items[1], StripItem::Page(_) | StripItem::Run { .. }) {
                break;
            }
            let ids = items.remove(1).ids().to_vec();
            items.insert(1, StripItem::Earlier { ids });
            continue;
        }
        let next = items.remove(2).ids().to_vec();
        if let StripItem::Earlier { ids } = &mut items[1] {
            ids.extend(next);
        }
    }
    let mut widths: Vec<_> = items
        .iter()
        .map(|item| item_width(forest, item, pill, current))
        .collect();
    let is_parent = |item: &StripItem| {
        matches!(item, StripItem::Parent(_))
            || matches!(item, StripItem::Root(id) if forest.visit(current).unwrap().parent == Some(*id))
    };
    let is_fixed = |item: &StripItem| !is_parent(item) && !matches!(item, StripItem::Current(_));
    let mut fixed: f64 = items
        .iter()
        .zip(&widths)
        .filter(|(item, _)| is_fixed(item))
        .map(|(_, w)| *w)
        .sum();
    let gaps = widths.len().saturating_sub(1) as f64;
    let mut gap = GAP;
    if least > 0. {
        // Pins and a long trail can leave less than a pill and the minimum shelf.
        let fixed_room = (room - PAD * 2. - least - NARROW_PILL.min(pill)).max(0.);
        if fixed > fixed_room {
            let scale = fixed_room / fixed;
            for (item, width) in items.iter().zip(&mut widths) {
                if is_fixed(item) {
                    *width *= scale;
                }
            }
            fixed = fixed_room;
        }
        if gaps > 0. {
            gap = GAP.min((fixed_room - fixed) / gaps);
        }
    }
    let free = room - PAD * 2. - fixed - gaps * gap;
    let parent = items
        .iter()
        .zip(&widths)
        .find(|(item, _)| is_parent(item))
        .map_or(0., |(_, w)| *w);
    let names: Vec<_> = waiting.pages()[..measured(free, waiting.len())]
        .iter()
        .map(|v| tab_width(&short_name(forest, v.id)))
        .collect();
    let fit = fit_shelf(&ShelfRoom {
        free,
        parent: Flex::new(parent, PARENT_FLOOR),
        pill: Flex::new(pill, PILL_FLOOR),
        names: &names,
        total: waiting.len(),
        below: waiting.below(),
    });
    let trail = place_trail(
        forest,
        &forest.path(current),
        items,
        widths,
        gap,
        is_parent,
        &fit,
    );
    let right = trail.last().map_or(PAD, |n| n.rect.x + n.rect.w / 2.);
    let shelf = place_shelf(waiting, fit.shelf, &names, right);
    StripLayout { trail, shelf }
}

fn place_trail(
    forest: &ForestProjection,
    path: &[VisitId],
    items: Vec<StripItem>,
    widths: Vec<f64>,
    gap: f64,
    is_parent: impl Fn(&StripItem) -> bool,
    fit: &ShelfFit,
) -> Vec<StripNode> {
    let mut left = PAD;
    items
        .into_iter()
        .zip(widths)
        .filter_map(|(item, w)| {
            let w = if is_parent(&item) {
                fit.parent
            } else if matches!(item, StripItem::Current(_)) {
                fit.pill
            } else {
                w
            };
            if w == 0. {
                return None;
            }
            let heads = branch_heads(forest, item.ids(), path);
            let branches = branch_counts(forest, &heads);
            let rows = match &item {
                StripItem::Run { ids, .. } => ids.clone(),
                StripItem::Earlier { ids } => ids.iter().rev().copied().collect(),
                StripItem::After { read } => read.clone(),
                StripItem::Current(_) => heads
                    .into_iter()
                    .filter(|id| forest.visit(*id).unwrap().opened_at.is_some())
                    .collect(),
                _ => heads,
            };
            let total_rows = rows.len();
            let s = match item {
                StripItem::Current(_) => 32.,
                StripItem::Root(_) | StripItem::Page(_) => 28.,
                _ => 24.,
            };
            let rect = Rect {
                x: left + w / 2.,
                y: TRUNK_Y,
                w,
                s,
            };
            left += w + gap;
            Some(StripNode {
                item,
                rect,
                branches,
                rows: rows.into_iter().take(6).collect(),
                total_rows,
            })
        })
        .collect()
}

fn place_shelf(waiting: &Waiting<'_>, shelf: Shelf, names: &[f64], right: f64) -> Vec<StripNode> {
    let (shown, named, hidden) = match shelf {
        Shelf::All { named } => (waiting.len(), named, None),
        Shelf::Cut { shown, hidden } => (shown, 0, Some(hidden)),
    };
    let mut nodes = Vec::with_capacity(shown + usize::from(hidden.is_some()));
    let mut left = right + GAP;
    let mut place = |item, w, divider| {
        if divider {
            left += DIVIDER_GAP - TAB_GAP;
        }
        let rect = Rect {
            x: left + w / 2.,
            y: TRUNK_Y,
            w,
            s: TAB,
        };
        nodes.push(StripNode {
            item,
            rect,
            branches: BranchCounts::default(),
            rows: vec![],
            total_rows: 0,
        });
        left += w + TAB_GAP;
    };
    for (i, page) in waiting.pages()[..shown].iter().enumerate() {
        let divider = divider_at(waiting.below(), shown) == Some(i);
        let named = i < named;
        let w = if named { names[i] } else { TAB };
        place(
            StripItem::Tab {
                id: page.id,
                below: i < waiting.below(),
                named,
                divider,
            },
            w,
            divider,
        );
    }
    if let Some(hidden) = hidden {
        place(
            StripItem::Overflow { hidden },
            overflow_width(hidden.get()),
            false,
        );
    }
    nodes
}

pub(crate) fn edge_kind(forest: &ForestProjection, id: VisitId) -> EdgeKind {
    if forest.is_waiting(id) {
        EdgeKind::Pending
    } else {
        EdgeKind::Plain
    }
}

pub(crate) fn edge_curve(edge: &Edge, a: Rect, b: Rect) -> [(f64, f64); 4] {
    let (ex, ey) = (b.x - b.w / 2., b.y);
    if edge.from_bottom {
        let (sx, sy) = (a.x, a.y + a.s / 2.);
        return [(sx, sy), (sx, ey), (sx + (ex - sx) * 0.3, ey), (ex, ey)];
    }
    let (sx, sy) = (a.x + a.w / 2., a.y);
    let dx = ((ex - sx) / 2.).max(5.);
    [(sx, sy), (sx + dx, sy), (ex - dx, ey), (ex, ey)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, TreeId, TreeState, Visit, VisitState};

    fn fixture(
        parents: &[Option<usize>],
        hosts: &[&str],
        queued: &[usize],
        current: usize,
    ) -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = parents
            .iter()
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let visits = parents
            .iter()
            .enumerate()
            .map(|(i, p)| Visit {
                dismissed_at: None,
                branch_settled_at: None,
                id: ids[i],
                parent: p.map(|p| ids[p]),
                url: format!("https://{}/{i}", hosts[i % hosts.len()])
                    .parse()
                    .unwrap(),
                title: format!("Tutorial · Step {i} - Docs"),
                created_at: i as i64,
                opened_at: (!queued.contains(&i)).then_some(i as i64),
                state: if queued.contains(&i) {
                    VisitState::Queued
                } else {
                    VisitState::Opened
                },
                source: None,
            })
            .collect();
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(ids[current]),
            },
            vec![TreeState {
                root: TreeId(ids[0]),
                last_visit: ids[current],

                name: None,
                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
            }],
        )
        .unwrap();
        (forest, ids)
    }
    fn chain(n: usize, hosts: &[&str]) -> (ForestProjection, Vec<VisitId>) {
        fixture(
            &(0..n).map(|i| i.checked_sub(1)).collect::<Vec<_>>(),
            hosts,
            &[],
            n - 1,
        )
    }
    fn assert_fits(nodes: &[StripNode], room: f64) {
        assert!(nodes
            .iter()
            .all(|n| n.rect.x - n.rect.w / 2. >= 0. && n.rect.x + n.rect.w / 2. <= room));
        assert!(nodes
            .windows(2)
            .all(|n| n[0].rect.x + n[0].rect.w / 2. < n[1].rect.x - n[1].rect.w / 2.));
    }
    #[test]
    fn same_site_chain_keeps_root_named_parent_and_full_current() {
        let (f, ids) = chain(8, &["docs.rs"]);
        let nodes = strip_layout(&f, ids[7], 712., 200., &Waiting::of(&f)).trail;
        assert!(matches!(nodes[0].item, StripItem::Root(id) if id == ids[0]));
        assert!(
            matches!(&nodes[1].item, StripItem::Run { ids: run, host } if run == &ids[1..6] && host == "docs.rs")
        );
        assert!(matches!(nodes[2].item, StripItem::Parent(id) if id == ids[6]));
        assert!(matches!(nodes[3].item, StripItem::Current(id) if id == ids[7]));
        assert_eq!(nodes[1].rows, ids[1..6]);
        assert_eq!(short_name(&f, ids[6]), "Step 6");
        assert_fits(&nodes, 712.);
    }
    #[test]
    fn fan_out_counts_entire_branches_and_lists_recent_heads() {
        let mut parents = vec![None, Some(0), Some(1)];
        parents.extend((0..12).map(|_| Some(0)));
        parents.push(Some(3));
        let (f, ids) = fixture(&parents, &["docs.rs"], &[4, 15], 2);
        let nodes = strip_layout(&f, ids[2], 712., 200., &Waiting::of(&f)).trail;
        assert_eq!(
            nodes[0].branches,
            BranchCounts {
                pages: 13,
                waiting: 2,
                read: 11,
            }
        );
        assert_eq!(nodes[0].total_rows, 12);
        assert_eq!(
            nodes[0].rows,
            ids[9..15].iter().rev().copied().collect::<Vec<_>>()
        );
    }
    #[test]
    fn deep_trail_merges_oldest_items_and_preserves_every_page() {
        let (f, ids) = chain(34, &["a.test", "b.test", "c.test"]);
        for room in [280., 390., 712., 1240.] {
            let nodes = strip_layout(&f, ids[33], room, 250., &Waiting::of(&f)).trail;
            assert_fits(&nodes, room);
            assert_eq!(
                nodes
                    .iter()
                    .flat_map(|n| n.item.ids())
                    .copied()
                    .collect::<Vec<_>>(),
                ids
            );
            let earlier = nodes
                .iter()
                .find(|n| matches!(n.item, StripItem::Earlier { .. }))
                .unwrap();
            assert!(matches!(&earlier.item, StripItem::Earlier { ids: old } if old[0] == ids[1]));
            assert_eq!(earlier.rows[0], *earlier.item.ids().last().unwrap());
            assert_eq!(earlier.rows.len(), 6);
        }
    }
    #[test]
    fn shelf_ladder_follows_the_width_boundaries() {
        let mut room = ShelfRoom {
            free: 1000.,
            parent: Flex::new(120., 60.),
            pill: Flex::new(250., 196.),
            names: &[90.; 4],
            total: 4,
            below: 2,
        };
        assert_eq!(
            fit_shelf(&room),
            ShelfFit {
                shelf: Shelf::All { named: 4 },
                parent: 120.,
                pill: 250.,
            }
        );
        for (free, at, below) in [
            (661., Shelf::All { named: 4 }, Shelf::All { named: 3 }),
            (599., Shelf::All { named: 3 }, Shelf::All { named: 2 }),
            (537., Shelf::All { named: 2 }, Shelf::All { named: 1 }),
            (475., Shelf::All { named: 1 }, Shelf::All { named: 0 }),
            (
                413.,
                Shelf::All { named: 0 },
                Shelf::Cut {
                    shown: 2,
                    hidden: NonZeroUsize::new(2).unwrap(),
                },
            ),
            (
                374.,
                Shelf::Cut {
                    shown: 2,
                    hidden: NonZeroUsize::new(2).unwrap(),
                },
                Shelf::Cut {
                    shown: 1,
                    hidden: NonZeroUsize::new(3).unwrap(),
                },
            ),
        ] {
            room.free = free;
            assert_eq!(fit_shelf(&room).shelf, at, "at {free}");
            room.free -= 1.;
            assert_eq!(fit_shelf(&room).shelf, below, "just below {free}");
        }
        let wide = ShelfRoom {
            free: 1200.,
            names: &[90.; 12],
            total: 12,
            ..room
        };
        let fit = fit_shelf(&wide);
        assert_eq!(fit.shelf, Shelf::All { named: 6 }, "spare room names more");
        assert_eq!((fit.parent, fit.pill), (120., 250.));
        room.free = 746.;
        assert_eq!(fit_shelf(&room).parent, 91.);
        assert_eq!(fit_shelf(&room).pill, 250.);
        assert_eq!(least_shelf(11) - least_shelf(10), 7.);
    }

    #[test]
    fn tabs_follow_the_waiting_order_and_fit_beside_a_long_trail() {
        let mut parents: Vec<_> = (0usize..15).map(|i| i.checked_sub(1)).collect();
        parents.extend([Some(14); 15]);
        parents.extend([Some(0); 15]);
        parents.push(Some(14));
        let (f, ids) = fixture(
            &parents,
            &["a.test", "b.test", "c.test"],
            &(15..45).collect::<Vec<_>>(),
            14,
        );
        let waiting = Waiting::of(&f);
        let expected: Vec<_> = ids[15..30]
            .iter()
            .rev()
            .chain(ids[30..45].iter().rev())
            .copied()
            .collect();
        for room in [227.5, 280., 390., 712., 1240., 3000.] {
            let layout = strip_layout(&f, ids[14], room, 250., &waiting);
            let trail = strip_layout(
                &f,
                ids[14],
                room - least_shelf(30),
                250.,
                &Waiting::default(),
            );
            assert_eq!(
                layout
                    .trail
                    .iter()
                    .find(|n| matches!(n.item, StripItem::Earlier { .. }))
                    .unwrap()
                    .item,
                trail
                    .trail
                    .iter()
                    .find(|n| matches!(n.item, StripItem::Earlier { .. }))
                    .unwrap()
                    .item,
            );
            assert!(layout
                .trail
                .iter()
                .any(|n| matches!(n.item, StripItem::Current(_))));
            assert!(
                layout.shelf.last().unwrap().rect.x + layout.shelf.last().unwrap().rect.w / 2.
                    <= room - PAD + 0.001
            );
            let shown: Vec<_> = layout
                .shelf
                .iter()
                .flat_map(|n| n.item.ids())
                .copied()
                .collect();
            assert_eq!(shown, expected[..shown.len()]);
            if let StripItem::Overflow { hidden } = layout.shelf.last().unwrap().item {
                assert_eq!(shown.len() + hidden.get(), expected.len());
                assert!(!shown.is_empty());
            }
            assert_eq!(
                layout
                    .shelf
                    .iter()
                    .filter(|n| matches!(n.item, StripItem::Tab { divider: true, .. }))
                    .count(),
                usize::from(shown.len() > 15)
            );
            for n in &layout.shelf {
                assert_eq!(n.rect.s, TAB);
                match n.item {
                    StripItem::Tab { named: true, .. } => assert!(n.rect.w <= TAB_MAX),
                    StripItem::Tab { named: false, .. } => assert_eq!(n.rect.w, TAB),
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn after_counts_read_children_and_waiting_children_become_tabs() {
        let mut parents = vec![None];
        parents.extend((0..18).map(|_| Some(0)));
        let queued: Vec<_> = (1..15).collect();
        let (f, ids) = fixture(&parents, &["news.test"], &queued, 0);
        let items = strip_items(&f, ids[0]);
        assert!(
            matches!(&items[1], StripItem::After { read } if read.as_slice() == ids[15..19].iter().rev().copied().collect::<Vec<_>>())
        );
        let nodes = strip_layout(&f, ids[0], 712., 250., &Waiting::of(&f)).trail;
        assert_eq!(
            nodes[0].rows,
            ids[15..19].iter().rev().copied().collect::<Vec<_>>()
        );
        assert_eq!(nodes[0].total_rows, 4);
        assert_eq!(nodes[1].rows, nodes[0].rows);
        assert_eq!(nodes[1].total_rows, 4);
        assert_fits(&nodes, 712.);
    }
    #[test]
    fn short_mixed_path_stays_unfolded_and_singletons_stay_tiles() {
        let (f, ids) = chain(5, &["a.test", "b.test"]);
        let nodes = strip_layout(&f, ids[4], 1240., 200., &Waiting::of(&f)).trail;
        assert!(matches!(nodes[1].item, StripItem::Page(id) if id == ids[1]));
        assert!(matches!(nodes[2].item, StripItem::Page(id) if id == ids[2]));
        assert!(!nodes
            .iter()
            .any(|n| matches!(n.item, StripItem::Run { .. } | StripItem::Earlier { .. })));
        assert_fits(&nodes, 1240.);
    }
    #[test]
    fn root_as_parent_is_named_once_and_narrow_runs_stay_runs() {
        let (f, ids) = chain(2, &["docs.rs"]);
        let nodes = strip_layout(&f, ids[1], 390., 250., &Waiting::of(&f)).trail;
        assert_eq!(nodes.len(), 2);
        assert!(nodes[0].rect.w > 28.);
        assert!(matches!(nodes[0].item, StripItem::Root(id) if id == ids[0]));
        assert_fits(&nodes, 390.);
        let (f, ids) = chain(8, &["docs.rs"]);
        let nodes = strip_layout(&f, ids[7], 280., 250., &Waiting::of(&f)).trail;
        assert!(matches!(nodes[1].item, StripItem::Run { .. }));
        assert_fits(&nodes, 280.);
    }

    #[test]
    fn roomy_trail_keeps_exactly_three_nearest_middle_items() {
        let (f, ids) = chain(34, &["a.test", "b.test", "c.test"]);
        for room in [820., 1240., 2000.] {
            let nodes = strip_layout(&f, ids[33], room, 250., &Waiting::of(&f)).trail;
            assert!(
                matches!(&nodes[1].item, StripItem::Earlier { ids: old } if old == &ids[1..29])
            );
            assert_eq!(
                nodes[2..5]
                    .iter()
                    .flat_map(|n| n.item.ids())
                    .copied()
                    .collect::<Vec<_>>(),
                ids[29..32]
            );
        }
    }

    #[test]
    fn parent_and_rows_keep_distinguishing_title_segments() {
        let (f, ids) = chain(4, &["docs.rs"]);
        assert_eq!(parent_name(&f, ids[2], ids[3]), "2");
        assert_eq!(
            distinct_name(
                "Ask HN · What are you building this month?",
                "Ask HN · What are you building next month?"
            ),
            "What are you building this month?"
        );
        assert_eq!(distinct_name("Same name", "Same name"), "Same name");
    }

    #[test]
    fn a_deep_same_site_history_has_a_bounded_list_without_recursion() {
        let (f, ids) = chain(20_000, &["docs.rs"]);
        let nodes = strip_layout(&f, ids[19_999], 1240., 250., &Waiting::of(&f)).trail;
        assert_eq!(nodes.len(), 4);
        assert_eq!(nodes[1].total_rows, 19_997);
        assert_eq!(nodes[1].rows.len(), 6);
        assert_fits(&nodes, 1240.);
    }
}
