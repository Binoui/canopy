use crate::forest::{ForestProjection, TreeSummary};
use serde_json::{json, Value};

/// The page the active tree is on, named the way the strip names it, when that differs
/// from the tree's own name. A YouTube tree shows the video being watched under "YouTube".
pub(super) fn subtitle(forest: &ForestProjection, tree: &TreeSummary) -> Option<String> {
    let current = forest.current()?;
    if forest.tree_of(current) != Some(tree.id) {
        return None;
    }
    let visit = forest.visit(current)?;
    let name = crate::naming::page_name(&visit.title, &visit.url);
    // "Tokio tutorial · Async in depth" under "Tokio tutorial · Getting started" reads
    // "Async in depth".
    let name = super::graph::distinct_name(&name, &tree.title);
    (!name.is_empty() && name != tree.title).then_some(name)
}

#[derive(Clone, Copy)]
enum Mode {
    Title,
    Icon,
    Hidden,
}

fn fit(count: usize, active: Option<usize>, natural: f64, room: f64) -> (Vec<(Mode, f64)>, bool) {
    if count == 0 {
        return (Vec::new(), false);
    }
    let icons = count - usize::from(active.is_some());
    // The tree being read is never narrower than the trees around it. It takes 200 px
    // first and grows toward its full name only into room no other title needs.
    let natural = natural.clamp(200., 280.);
    let base = 200_f64;
    let overflow = icons > 0
        && (if active.is_some() { base } else { 0. })
            + icons as f64 * 34.
            + (count - 1) as f64 * 2.
            > room;
    let room = (room - if overflow { 32. } else { 0. }).max(0.);
    let mut active_width = base.min(room);
    let mut result = vec![(Mode::Title, 70.); count];
    if let Some(i) = active {
        result[i].1 = active_width;
    }
    let used = |items: &[(Mode, f64)]| {
        let shown = items
            .iter()
            .filter(|(m, _)| !matches!(m, Mode::Hidden))
            .count();
        items.iter().map(|(_, w)| w).sum::<f64>() + shown.saturating_sub(1) as f64 * 2.
    };
    let mut farthest: Vec<_> = (0..count).filter(|i| Some(*i) != active).collect();
    farthest.sort_by_key(|i| std::cmp::Reverse((i.abs_diff(active.unwrap_or(0)), *i)));
    for &i in &farthest {
        if used(&result) <= room {
            break;
        }
        result[i] = (Mode::Icon, 34.);
    }
    for &i in &farthest {
        if used(&result) <= room {
            break;
        }
        result[i] = (Mode::Hidden, 0.);
    }
    if let Some(i) = active {
        active_width += (natural - active_width).min(room - used(&result)).max(0.);
        result[i].1 = active_width;
    }
    let titles = result
        .iter()
        .enumerate()
        .filter(|(i, (m, _))| Some(*i) != active && matches!(m, Mode::Title))
        .count();
    if titles > 0 {
        let extra =
            ((room - used(&result)) / titles as f64).clamp(0., active_width.max(base) - 70.);
        for (i, (mode, width)) in result.iter_mut().enumerate() {
            if Some(i) != active && matches!(mode, Mode::Title) {
                *width += extra;
            }
        }
    }
    (result, overflow)
}

/// Pinned trees lead the row. They keep their names while every tab has room for a full
/// one and fold to their icons first when room runs short; the tree being read keeps its
/// name either way.
fn fit_pinned(
    count: usize,
    pins: usize,
    active: Option<usize>,
    natural: f64,
    room: f64,
) -> (Vec<(Mode, f64)>, bool) {
    if pins == 0 {
        return fit(count, active, natural, room);
    }
    let named = fit(count, active, natural, room);
    if !named.1
        && named
            .0
            .iter()
            .all(|&(mode, width)| matches!(mode, Mode::Title) && width >= 160.)
    {
        return named;
    }
    let icons = (0..pins).filter(|&i| Some(i) != active).count();
    let reserve = icons as f64 * 36.;
    let rest: Vec<usize> = (0..count)
        .filter(|&i| i >= pins || Some(i) == active)
        .collect();
    let need_current = if active.is_some() { 34. } else { 0. };
    // The overflow button needs room only when unpinned trees may not all fit.
    let need_overflow = if rest.len() > usize::from(active.is_some()) {
        32.
    } else {
        -2.
    };
    if room >= reserve + need_current + need_overflow {
        let (fitted, overflow) = fit(
            rest.len(),
            active.and_then(|a| rest.iter().position(|&i| i == a)),
            natural,
            (room - reserve).max(0.),
        );
        let mut items = vec![(Mode::Icon, 34.); count];
        for (&i, item) in rest.iter().zip(fitted) {
            items[i] = item;
        }
        return (items, overflow);
    }
    let mut items = vec![(Mode::Hidden, 0.); count];
    let capacity = (((room - 32.).max(0.) + 2.) / 36.).floor() as usize;
    let mut selected = active.into_iter().chain(0..count).collect::<Vec<_>>();
    selected.dedup();
    let mut shown = 0;
    for i in selected {
        if !matches!(items[i].0, Mode::Hidden) {
            continue;
        }
        if shown >= capacity.max(usize::from(active.is_some())) {
            break;
        }
        items[i] = (Mode::Icon, 34.);
        shown += 1;
    }
    (items, true)
}

pub(super) fn layout(forest: &ForestProjection, room: f64, start: bool) -> Value {
    let current = forest
        .current()
        .filter(|_| !start)
        .and_then(|id| forest.tree_of(id));
    let active = forest.tabs().iter().position(|t| Some(t.id) == current);
    let (items, overflow) = fit_pinned(
        forest.tabs().len(),
        forest.tabs().partition_point(|t| t.place.pinned()),
        active,
        active
            .map(|i| {
                let tree = &forest.tabs()[i];
                let sub =
                    subtitle(forest, tree).map_or(0., |s| super::view::text_width(&s, 11., 0.53));
                let title = super::view::text_width(&tree.title, 12.5, 0.56).max(sub) + 64.;
                // The pin mark after a pinned tree's name.
                title + if tree.place.pinned() { 17. } else { 0. }
            })
            .unwrap_or(110.),
        room,
    );
    json!({
        "overflow": overflow,
        "items": forest.tabs().iter().zip(items).map(|(tree, (mode, width))| {
            json!({"id": tree.id.root().to_string(), "mode": match mode { Mode::Title => "title", Mode::Icon => "icon", Mode::Hidden => "hidden" }, "width": width, "short": width < 160.})
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pinned_icons_stay_first_and_current_survives_pin_overflow() {
        for room in [1000., 600., 300., 150., 60.] {
            let (tabs, overflow) = fit_pinned(20, 4, Some(10), 200., room);
            assert!(!matches!(tabs[10].0, Mode::Hidden));
            if room >= 4. * 36. + 34. + 32. {
                for t in &tabs[..4] {
                    assert!(matches!(t.0, Mode::Icon));
                    assert_eq!(t.1, 34.);
                }
            }
            if room < 20. * 36. {
                assert!(overflow);
            }
        }
        // The pinned tree being read keeps its name while the other pins fold.
        let (tabs, _) = fit_pinned(8, 4, Some(2), 200., 800.);
        assert!(matches!(tabs[2].0, Mode::Title));
        for i in [0, 1, 3] {
            assert!(matches!(tabs[i].0, Mode::Icon));
        }
    }

    #[test]
    fn pins_keep_their_names_while_there_is_room() {
        // Two trees, one pinned, in a wide window: both named.
        let (tabs, overflow) = fit_pinned(2, 1, Some(1), 240., 1200.);
        assert!(!overflow);
        assert!(tabs
            .iter()
            .all(|t| matches!(t.0, Mode::Title) && t.1 >= 160.));
        // Short of room, the pin folds to its icon before any other tree loses its name.
        let (tabs, _) = fit_pinned(6, 1, Some(5), 240., 900.);
        assert!(matches!(tabs[0].0, Mode::Icon));
        assert!(tabs[1..].iter().all(|t| matches!(t.0, Mode::Title)));
        // Only pins, no room for names: icons.
        let (tabs, overflow) = fit_pinned(3, 3, None, 200., 300.);
        assert!(!overflow);
        assert!(tabs.iter().all(|t| matches!(t.0, Mode::Icon)));
        let (tabs, overflow) = fit_pinned(3, 3, None, 200., 106.);
        assert!(!overflow);
        assert!(tabs.iter().all(|t| matches!(t.0, Mode::Icon)));
    }
    #[test]
    fn active_title_gets_room_before_other_trees() {
        for room in [900., 600., 300., 240., 120., 40.] {
            let (tabs, overflow) = fit(16, Some(8), 185., room);
            // 185 px of name still gets the 200 px floor.
            assert_eq!(
                tabs[8].1,
                200_f64.min((room - if overflow { 32. } else { 0. }).max(0.))
            );
            let shown: Vec<_> = tabs
                .iter()
                .filter(|(m, _)| !matches!(m, Mode::Hidden))
                .collect();
            assert!(
                shown.iter().map(|(_, w)| w).sum::<f64>()
                    + (shown.len() - 1) as f64 * 2.
                    + if overflow { 32. } else { 0. }
                    <= room + 0.01
            );
        }
    }

    #[test]
    fn active_tab_is_never_narrower_than_the_others() {
        for natural in [80., 150., 240., 400.] {
            let (tabs, _) = fit(3, Some(1), natural, 1400.);
            assert!(tabs[1].1 >= 200. && tabs[1].1 <= 280.);
            assert!(tabs.iter().all(|(_, w)| *w <= tabs[1].1));
        }
    }

    #[test]
    fn titles_use_spare_room_and_collapse_from_the_far_end() {
        let (tabs, overflow) = fit(4, Some(0), 200., 560.);
        assert!(!overflow);
        assert!(tabs.iter().all(|(m, _)| matches!(m, Mode::Title)));
        assert_eq!(tabs[0].1, 200.);
        let (tabs, overflow) = fit(4, Some(0), 200., 400.);
        assert!(!overflow);
        assert!(matches!(tabs[1].0, Mode::Title));
        assert!(matches!(tabs[2].0, Mode::Title));
        assert!(matches!(tabs[3].0, Mode::Icon));
    }
}
