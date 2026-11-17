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

pub(super) fn layout(forest: &ForestProjection, room: f64) -> Value {
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let active = forest.open().iter().position(|t| Some(t.id) == current);
    let (items, overflow) = fit(
        forest.open().len(),
        active,
        active
            .map(|i| {
                let tree = &forest.open()[i];
                let sub =
                    subtitle(forest, tree).map_or(0., |s| super::view::text_width(&s, 11., 0.53));
                let title = super::view::text_width(&tree.title, 12.5, 0.56).max(sub) + 64.;
                title
                    + if tree.pin.is_some() && title >= 160. {
                        17.
                    } else {
                        0.
                    }
            })
            .unwrap_or(110.),
        room,
    );
    json!({
        "overflow": overflow,
        "items": forest.open().iter().zip(items).map(|(tree, (mode, width))| {
            json!({"id": tree.id.root().to_string(), "mode": match mode { Mode::Title => "title", Mode::Icon => "icon", Mode::Hidden => "hidden" }, "width": width, "short": width < 160.})
        }).collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
