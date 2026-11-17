use crate::forest::ForestProjection;
use serde_json::{json, Value};

#[derive(Clone, Copy)]
enum Mode {
    Title,
    Icon,
    Hidden,
}

fn fit(count: usize, active: Option<usize>, room: f64) -> (Vec<(Mode, f64)>, bool) {
    if count == 0 {
        return (Vec::new(), false);
    }
    let width = ((room - (count - 1) as f64 * 2.) / count as f64).min(200.);
    if width >= 110. {
        return (vec![(Mode::Title, width); count], false);
    }
    let icons = count - usize::from(active.is_some());
    let active_width = if active.is_some() {
        (room - icons as f64 * 36.).clamp(110., 160.)
    } else {
        0.
    };
    let need = active_width + icons as f64 * 34. + (count - 1) as f64 * 2.;
    let overflow = need > room && icons > 0;
    let room = (room - if overflow { 32. } else { 0. }).max(0.);
    let title_width = active_width.min(room);
    let mut remaining = (room - title_width - if active.is_some() { 2. } else { 0. }).max(0.);
    let mut result = vec![(Mode::Hidden, 0.); count];
    for (i, item) in result.iter_mut().enumerate() {
        if Some(i) == active {
            *item = (Mode::Title, title_width);
        } else if remaining >= 34. {
            *item = (Mode::Icon, 34.);
            remaining -= 36.;
        }
    }
    (result, overflow)
}

pub(super) fn layout(forest: &ForestProjection, room: f64) -> Value {
    let current = forest.current().and_then(|id| forest.tree_of(id));
    let active = forest.open().iter().position(|t| Some(t.id) == current);
    let (items, overflow) = fit(forest.open().len(), active, room);
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
    fn shrink_titles_then_icons_then_overflow_and_cap_the_active() {
        let (single, overflow) = fit(1, Some(0), 40.);
        assert!(!overflow);
        assert_eq!(single[0].1, 40.);
        for room in [2400., 1800.] {
            let (tabs, overflow) = fit(16, Some(15), room);
            assert!(!overflow);
            assert!(tabs
                .iter()
                .all(|(m, w)| matches!(m, Mode::Title) && *w >= 110.));
        }
        let (tabs, overflow) = fit(16, Some(15), 750.);
        assert!(!overflow);
        assert!(matches!(tabs[0].0, Mode::Icon));
        assert!(matches!(tabs[15].0, Mode::Title));
        for room in [600., 300., 120., 40.] {
            let (tabs, overflow) = fit(16, Some(15), room);
            assert!(overflow);
            assert!(matches!(tabs[15].0, Mode::Title));
            let shown: Vec<_> = tabs
                .iter()
                .filter(|(m, _)| !matches!(m, Mode::Hidden))
                .collect();
            assert!(
                shown.iter().map(|(_, w)| w).sum::<f64>() + (shown.len() - 1) as f64 * 2. + 32.
                    <= room
            );
        }
    }
}
