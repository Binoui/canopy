use crate::forest::ForestProjection;
use serde_json::{json, Value};

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
    let natural = natural.clamp(110., 200.);
    let overflow = icons > 0
        && (if active.is_some() { natural } else { 0. })
            + icons as f64 * 34.
            + (count - 1) as f64 * 2.
            > room;
    let room = (room - if overflow { 32. } else { 0. }).max(0.);
    let active_width = natural.min(room);
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
    let titles = result
        .iter()
        .enumerate()
        .filter(|(i, (m, _))| Some(*i) != active && matches!(m, Mode::Title))
        .count();
    if titles > 0 {
        let extra = ((room - used(&result)) / titles as f64).clamp(0., 130.);
        for (i, (mode, width)) in result.iter_mut().enumerate() {
            if Some(i) != active && matches!(mode, Mode::Title) {
                *width += extra;
            }
        }
    }
    (result, overflow)
}

pub(super) fn layout(forest: &ForestProjection, room: f64, start: bool) -> Value {
    let current = forest
        .current()
        .filter(|_| !start)
        .and_then(|id| forest.tree_of(id));
    let active = forest.open().iter().position(|t| Some(t.id) == current);
    let (items, overflow) = fit(
        forest.open().len(),
        active,
        active
            .map(|i| {
                let tree = &forest.open()[i];
                let title = super::view::text_width(&tree.title, 12.5, 0.56) + 64.;
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
            assert_eq!(
                tabs[8].1,
                185_f64.min((room - if overflow { 32. } else { 0. }).max(0.))
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
