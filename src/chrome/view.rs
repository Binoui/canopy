//! What the chrome view draws, as JSON. Rust decides every position and every string; the
//! HTML view only paints them and reports input back.

use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use super::{
    forest_layout,
    graph::{self, EdgeKind, NodeKind, StripLayout},
    palette::{Group, PaletteItem},
    ChromeFrame,
};
use crate::{
    browser::{Capture, CaptureKind, ThemeMode, Visit, VisitId, VisitPreview, VisitState},
    forest::ForestProjection,
};

/// Room at the end of the current-page pill for the Memory dot.
const ACCESSORY: f64 = 22.;

pub(crate) fn frame(frame: &ChromeFrame, width: f64) -> Value {
    let forest = &frame.forest;
    let current = forest.current();
    let active = current.and_then(|id| forest.tree_of(id));
    let trees: Vec<Value> = forest
        .trees()
        .iter()
        .map(|tree| {
            json!({
                "id": tree.id.root().to_string(),
                "title": tree.title,
                "pages": tree.page_count,
                "pending": tree.pending_count,
                "active": Some(tree.id) == active,
            })
        })
        .collect();
    let mut visits = serde_json::Map::new();
    for tree in forest.trees() {
        let mut stack = vec![tree.id.root()];
        while let Some(id) = stack.pop() {
            if let Some(visit) = forest.visit(id) {
                visits.insert(id.to_string(), visit_value(forest, visit, &tree.title));
            }
            stack.extend(forest.children(id).iter().copied());
        }
    }
    let pages: usize = forest.trees().iter().map(|tree| tree.page_count).sum();
    json!({
        "current": current.map(|id| id.to_string()),
        "trees": trees,
        "visits": visits,
        "strip": strip(forest, width),
        "forest": {
            "count": format!("{} · {}", plural(forest.trees().len(), "tree"), plural(pages, "page")),
            "labels": forest_value(forest, true),
            "icons": forest_value(forest, false),
        },
        "captureEnabled": frame.capture_enabled,
        "theme": match frame.theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        },
        "shortcuts": frame.suggestions.shortcuts.iter().enumerate().map(|(index, shortcut)| {
            let host = short_host(&shortcut.url);
            json!({
                "index": index,
                "url": shortcut.url.as_str(),
                "label": if shortcut.title.is_empty() { host } else { shortcut.title.as_str() },
            })
        }).collect::<Vec<_>>(),
        "paths": frame.suggestions.paths.iter().enumerate().map(|(index, path)| json!({
            "index": index,
            "first": path.first.as_str(),
            "second": path.second.as_str(),
            "text": format!("{}, then {}", short_host(&path.first), short_host(&path.second)),
            "count": format!("{} times", path.visits),
        })).collect::<Vec<_>>(),
    })
}

/// The current tree's strip at rest and open, for a window `width` wide.
pub(crate) fn strip(forest: &ForestProjection, width: f64) -> Value {
    let Some(current) = forest.current() else {
        return json!({
            "rest": empty_strip(graph::REST_HEIGHT),
            "open": empty_strip(graph::OPEN_HEIGHT),
        });
    };
    let pill = forest.visit(current).map_or(120., pill_width);
    let layout = |open| {
        let layout = graph::strip_layout(forest, current, width, open, pill);
        strip_value(&layout, width)
    };
    json!({ "rest": layout(false), "open": layout(true) })
}

fn empty_strip(height: f64) -> Value {
    json!({ "height": height, "width": 0, "pan": 0, "nodes": [], "edges": [] })
}

fn strip_value(layout: &StripLayout, width: f64) -> Value {
    let nodes: Vec<Value> = layout
        .nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id.to_string(),
                "x": node.rect.x, "y": node.rect.y, "w": node.rect.w, "s": node.rect.s,
                "kind": match node.kind {
                    NodeKind::Pill => "pill",
                    NodeKind::Tile => "tile",
                    NodeKind::Raw => "raw",
                    NodeKind::Bud => "bud",
                },
                "path": node.on_path,
                "hidden": node.hidden.iter().map(ToString::to_string).collect::<Vec<_>>(),
            })
        })
        .collect();
    let edges: Vec<Value> = layout
        .edges
        .iter()
        .filter_map(|edge| {
            let (a, b) = (layout.node(edge.from)?, layout.node(edge.to)?);
            Some(json!({
                "from": edge.from.to_string(),
                "to": edge.to.to_string(),
                "kind": match edge.kind {
                    EdgeKind::Path => "path",
                    EdgeKind::Pending => "pending",
                    EdgeKind::Plain => "plain",
                },
                "c": graph::edge_curve(edge, a.rect, b.rect),
            }))
        })
        .collect();
    json!({
        "height": layout.height,
        "width": layout.width,
        "pan": graph::pan(layout, width),
        "trunk": layout.trunk().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "nodes": nodes,
        "edges": edges,
    })
}

fn forest_value(forest: &ForestProjection, labels: bool) -> Value {
    let layout = forest_layout::layout(forest, labels, |text| text_width(text, 12.5, 0.55));
    let current_tree = forest.current().and_then(|id| forest.tree_of(id));
    json!({
        "width": layout.current_width,
        "nodes": layout.nodes.iter().map(|(id, r)| json!({
            "id": id.to_string(), "x": r.x, "y": r.y, "w": r.w, "s": r.s,
        })).collect::<Vec<_>>(),
        "heads": layout.heads.iter().map(|&(index, y)| {
            let tree = &forest.trees()[index];
            json!({
                "y": y,
                "title": tree.title,
                "detail": format!(
                    "{} · {} pending{}",
                    plural(tree.page_count, "page"),
                    tree.pending_count,
                    if Some(tree.id) == current_tree { " · Current tree" } else { "" },
                ),
            })
        }).collect::<Vec<_>>(),
    })
}

fn visit_value(forest: &ForestProjection, visit: &Visit, tree: &str) -> Value {
    let source = visit.source.as_ref().map(|source| {
        let host = source.page_url.host_str().unwrap_or("the source");
        json!({
            "text": if source.link_text.is_empty() {
                format!("From {host}")
            } else {
                format!("From {host} · {}", source.link_text)
            },
            "citation": source.citation_url.is_some(),
        })
    });
    json!({
        "id": visit.id.to_string(),
        "parent": visit.parent.map(|id| id.to_string()),
        "url": visit.url.as_str(),
        "label": visit.label(),
        "site": site_line(visit),
        "queued": visit.state == VisitState::Queued,
        "after": forest.descendant_count(visit.id).unwrap_or(0),
        "tree": tree,
        "source": source,
    })
}

/// The palette's rows, in order. `item` indexes the results Rust keeps for running.
pub(crate) fn palette_rows(items: &[(Group, PaletteItem)], forest: &ForestProjection) -> Value {
    let rows: Vec<Value> = items
        .iter()
        .enumerate()
        .filter_map(|(index, (group, item))| {
            let (kind, title, detail, icon) = match item {
                PaletteItem::Page(id) => {
                    let visit = forest.visit(*id)?;
                    let tree = forest
                        .tree_of(*id)
                        .and_then(|tree| forest.trees().iter().find(|t| t.id == tree))
                        .map(|tree| tree.title.clone())
                        .unwrap_or_default();
                    let pending = if visit.state == VisitState::Queued { " · pending" } else { "" };
                    let icon = json!({ "url": visit.url.as_str(), "opened": visit.state == VisitState::Opened });
                    ("page", visit.label().to_owned(), format!("{tree}{pending}"), icon)
                }
                PaletteItem::Saved { title, excerpt, .. } => {
                    let excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
                    ("saved", title.clone(), excerpt.chars().take(140).collect(), json!("search"))
                }
                PaletteItem::Path(path) => (
                    "path",
                    format!("{} → {}", short_host(&path.first), short_host(&path.second)),
                    format!("{} visits", path.visits),
                    json!({ "url": path.first.as_str(), "opened": true }),
                ),
                PaletteItem::Shortcut(url) => (
                    "shortcut",
                    short_host(url).to_owned(),
                    url.path().to_owned(),
                    json!({ "url": url.as_str(), "opened": true }),
                ),
                PaletteItem::Go(url) => ("go", format!("Go to {url}"), String::new(), json!("globe")),
                PaletteItem::WebSearch { query, .. } => (
                    "search",
                    format!("Search the web for “{query}”"),
                    "DuckDuckGo".to_owned(),
                    json!("search"),
                ),
            };
            Some(json!({
                "item": index,
                "group": group.title(),
                "kind": kind,
                "title": title,
                "detail": detail,
                "icon": icon,
            }))
        })
        .collect();
    Value::Array(rows)
}

pub(crate) fn preview(forest: &ForestProjection, id: VisitId, preview: &VisitPreview) -> Value {
    let snippet = preview
        .snippet
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let after = forest.descendant_count(id).unwrap_or(0);
    let meta = match forest.visit(id) {
        Some(visit) if after > 0 => {
            format!("{} · {} after", site_line(visit), plural(after, "page"))
        }
        Some(visit) => site_line(visit),
        None => String::new(),
    };
    json!({ "id": id.to_string(), "title": preview.title, "meta": meta, "snippet": snippet })
}

/// One saved text for the reader, with what its actions can reach.
pub(crate) fn capture(capture: &Capture, visit: &Visit) -> Value {
    json!({
        "capture": capture.id,
        "visit": visit.id.to_string(),
        "parent": visit.parent.map(|id| id.to_string()),
        "citation": visit.source.as_ref().and_then(|s| s.citation_url.as_ref()).is_some(),
        "title": capture.title,
        "kind": if capture.kind == CaptureKind::Selection { "Selection" } else { "Page" },
        "at": capture.captured_at,
        "url": capture.document_url.as_str(),
        "source": capture.source.as_ref().map(|source| json!({
            "url": source.page_url.as_str(),
            "linkText": source.link_text,
            "excerpt": source.excerpt,
        })),
        "text": capture.text,
    })
}

pub(crate) fn captures(captures: &[Capture]) -> Value {
    Value::Array(
        captures
            .iter()
            .map(|c| json!({ "capture": c.id, "title": c.title, "at": c.captured_at }))
            .collect(),
    )
}

/// The current-page pill fits its title or site line, whichever is wider, plus the dot.
fn pill_width(visit: &Visit) -> f64 {
    let text = text_width(visit.label(), 12.5, 0.56).max(text_width(&site_line(visit), 11., 0.53));
    (7. + 16. + 8. + text + 16.).min(250.) + ACCESSORY
}

/// Inter's average advance, close enough for layout; the view ellipsizes the rest.
fn text_width(text: &str, size: f64, em: f64) -> f64 {
    text.chars().count() as f64 * size * em
}

pub(crate) fn site_line(visit: &Visit) -> String {
    let host = short_host(&visit.url);
    match visit.state {
        VisitState::Queued => format!("{host} · pending"),
        VisitState::Opened => format!(
            "{host} · {}",
            ago(visit.opened_at.unwrap_or(visit.created_at))
        ),
    }
}

fn short_host(url: &url::Url) -> &str {
    let host = url.host_str().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host)
}

fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

pub(crate) fn ago(timestamp_ms: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let minutes = (now - timestamp_ms).max(0) / 60_000;
    match minutes {
        0 => "just now".into(),
        1..=59 => format!("{minutes} min ago"),
        60..=1439 => format!("{} h ago", (minutes + 30) / 60),
        _ => match (minutes + 720) / 1440 {
            1 => "yesterday".into(),
            days => format!("{days} days ago"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::{Snapshot, StartSuggestions, TreeId, TreeState};

    fn chain() -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = (0..3)
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let visits = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| Visit {
                id,
                parent: i.checked_sub(1).map(|p| ids[p]),
                url: url::Url::parse(&format!("https://www.example.org/{i}")).unwrap(),
                title: format!("Page {i}"),
                created_at: 0,
                opened_at: Some(0),
                state: if i == 2 {
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
                current: Some(ids[1]),
            },
            vec![TreeState {
                root: TreeId(ids[0]),
                last_visit: ids[1],
            }],
        )
        .unwrap();
        (forest, ids)
    }

    #[test]
    fn the_frame_carries_every_visit_and_both_strip_states() {
        let (forest, ids) = chain();
        let frame = ChromeFrame {
            forest,
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
        };
        let value = super::frame(&frame, 1240.);
        assert_eq!(value["current"], ids[1].to_string());
        assert_eq!(value["visits"].as_object().unwrap().len(), 3);
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], true);
        assert_eq!(
            value["visits"][ids[0].to_string()]["site"],
            format!("example.org · {}", ago(0))
        );
        assert_eq!(value["trees"][0]["pending"], 1);
        assert_eq!(value["strip"]["rest"]["height"], graph::REST_HEIGHT);
        assert_eq!(value["strip"]["open"]["height"], graph::OPEN_HEIGHT);
        let pill = value["strip"]["rest"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "pill")
            .unwrap();
        assert_eq!(pill["id"], ids[1].to_string());
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["forest"]["count"], "1 tree · 3 pages");
    }

    #[test]
    fn edges_carry_four_control_points() {
        let (forest, _) = chain();
        let strip = strip(&forest, 1240.);
        let edge = &strip["open"]["edges"][0];
        assert_eq!(edge["c"].as_array().unwrap().len(), 4);
        assert_eq!(edge["c"][0].as_array().unwrap().len(), 2);
    }
}
