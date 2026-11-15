//! What the chrome view draws, as JSON. Rust decides every position and every string; the
//! HTML view only paints them and reports input back.

use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use super::{
    forest_layout,
    graph::{self, EdgeKind, NodeKind, StripLayout},
    palette::{Group, PaletteItem},
    pins,
    waiting::{self, WaitState, WaitingOrder},
    ChromeFrame, PageFile,
};
use crate::{
    browser::{
        Capture, CaptureKind, Lifecycle, ThemeMode, Visit, VisitId, VisitPreview, VisitState,
    },
    forest::ForestProjection,
};

/// Room at the end of the current-page pill for the Memory dot.
const ACCESSORY: f64 = 22.;

pub(crate) fn frame(frame: &ChromeFrame, width: f64) -> Value {
    let forest = &frame.forest;
    let current = forest.current();
    let active = current.and_then(|id| forest.tree_of(id));
    let tabs: Vec<Value> = forest
        .open()
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
                visits.insert(
                    id.to_string(),
                    visit_value(forest, visit, &tree.title, &frame.loaded),
                );
            }
            stack.extend(forest.children(id).iter().copied());
        }
    }
    let pages: usize = forest.trees().iter().map(|tree| tree.page_count).sum();
    json!({
        "current": current.map(|id| id.to_string()),
        "tabs": tabs,
        "visits": visits,
        "strip": strip(forest, width),
        "waiting": { "order": WaitingOrder::CHOSEN.label(), "ids": waiting::pages(forest).iter().map(|v| v.id.to_string()).collect::<Vec<_>>() },
        "files": files_value(&frame.files),
        "forest": {
            "count": if forest.settled().is_empty() {
                format!("{} · {}", plural(forest.trees().len(), "tree"), plural(pages, "page"))
            } else {
                format!("{} open · {} settled · {}", forest.open().len(), forest.settled().len(), plural(pages, "page"))
            },
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
    let pins = pins::pins(forest, width);
    let room = pins.room;
    let placed: Vec<_> = pins
        .placed
        .iter()
        .map(|(pin, r)| json!({ "kind": pin.kind(), "x": r.x, "y": r.y, "w": r.w, "s": r.s }))
        .collect();
    let Some(current) = forest.current() else {
        return json!({
            "room": room, "pins": placed,
            "rest": empty_strip(graph::REST_HEIGHT),
            "open": empty_strip(graph::OPEN_HEIGHT),
        });
    };
    let pill = forest.visit(current).map_or(120., pill_width);
    let layout = |open| {
        let layout = graph::strip_layout(forest, current, room, open, pill);
        strip_value(&layout, room)
    };
    json!({ "rest": layout(false), "open": layout(true), "room": room, "pins": placed })
}

fn empty_strip(height: f64) -> Value {
    json!({ "height": height, "width": 0, "pan": 0, "nodes": [], "edges": [] })
}

fn strip_value(layout: &StripLayout, room: f64) -> Value {
    let width = layout
        .nodes
        .iter()
        .map(|n| n.rect.x + n.rect.w / 2. + 22.)
        .fold(layout.width, f64::max);
    let mut pan = graph::pan(layout, room).clamp((room - width).min(0.), 0.);
    if let Some(pill) = layout.nodes.iter().find(|n| n.kind == NodeKind::Pill) {
        let left = pill.rect.x - pill.rect.w / 2.;
        let right = pill.rect.x + pill.rect.w / 2.;
        let lower = (-left).max((room - width).min(0.));
        let upper = (room - right).min(0.);
        if lower <= upper {
            pan = pan.clamp(lower, upper);
        }
    }

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
        "width": width,
        "pan": pan,
        "trunk": layout.trunk().iter().map(ToString::to_string).collect::<Vec<_>>(),
        "nodes": nodes,
        "edges": edges,
    })
}

/// Downloaded files by the page they hang off, oldest first.
fn files_value(files: &[PageFile]) -> Value {
    let mut pages = serde_json::Map::new();
    for file in files {
        let download = &file.download;
        let folder = download
            .path
            .parent()
            .and_then(|folder| folder.file_name())
            .map_or_else(|| "the folder".into(), |name| name.to_string_lossy());
        let detail = if file.missing {
            "Moved or deleted".to_owned()
        } else {
            format!("{} · {}", size_text(download.size), ago(download.saved_at))
        };
        let entry = json!({
            "id": download.id,
            "name": download.name(),
            "detail": detail,
            "folder": folder,
            "missing": file.missing,
        });
        if let Value::Array(entries) = pages
            .entry(download.visit.to_string())
            .or_insert_with(|| json!([]))
        {
            entries.push(entry);
        }
    }
    Value::Object(pages)
}

/// A file size the way file managers write it, in decimal units.
fn size_text(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.;
    let mut unit = 0;
    while value >= 999.5 && unit + 1 < UNITS.len() {
        value /= 1000.;
        unit += 1;
    }
    if value < 9.95 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

fn forest_value(forest: &ForestProjection, labels: bool) -> Value {
    let layout = forest_layout::layout(forest, labels, |text| text_width(text, 12.5, 0.55));
    let current_tree = forest.current().and_then(|id| forest.tree_of(id));
    let settled: HashSet<_> = forest.settled().iter().map(|tree| tree.id).collect();
    json!({
        "width": layout.current_width,
        "divider": layout.divider.map(|y| json!({
            "y": y, "title": "Settled", "detail": plural(forest.settled().len(), "tree"),
        })),
        "nodes": layout.nodes.iter().map(|(id, r)| json!({
            "settled": forest.tree_of(*id).is_some_and(|tree| settled.contains(&tree)),
            "id": id.to_string(), "x": r.x, "y": r.y, "w": r.w, "s": r.s,
        })).collect::<Vec<_>>(),
        "heads": layout.heads.iter().map(|&(index, y)| {
            let tree = &forest.trees()[index];
            json!({
                "y": y,
                "id": tree.last_visit.to_string(),
                "title": tree.title,
                "settled": !tree.lifecycle.is_open(),
                "detail": format!(
                    "{} · {}{}",
                    plural(tree.page_count, "page"),
                    match tree.lifecycle {
                        Lifecycle::Open => format!("{} pending", tree.pending_count),
                        Lifecycle::Settled { since } => format!("settled {}", ago(since)),
                    },
                    if Some(tree.id) == current_tree { " · Current tree" } else { "" },
                ),
            })
        }).collect::<Vec<_>>(),
    })
}

fn visit_value(
    forest: &ForestProjection,
    visit: &Visit,
    tree: &str,
    loaded: &HashSet<VisitId>,
) -> Value {
    let wait = WaitState::of(visit, loaded);
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
        "loaded": wait == Some(WaitState::Loaded),
        "when": match wait { Some(WaitState::NotLoaded) => "not loaded".into(), Some(WaitState::Loaded) => "loaded".into(), None => ago(visit.opened_at.unwrap_or(visit.created_at)) },
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
                PaletteItem::Imported(page) => {
                    let host = short_host(&page.url).to_owned();
                    let title = if page.title.trim().is_empty() { host.clone() } else { page.title.clone() };
                    let (kind, detail) = match &page.folder {
                        Some(folder) => ("bookmark", folder.clone()),
                        None if page.visits > 0 => ("imported", format!("{host} · {}", plural(page.visits, "visit"))),
                        None => ("imported", host),
                    };
                    (kind, title, detail, json!({ "url": page.url.as_str(), "opened": true }))
                }
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
    use crate::browser::{Download, Snapshot, StartSuggestions, TreeId, TreeState};

    fn chain() -> (ForestProjection, Vec<VisitId>) {
        chain_with_lifecycle(Lifecycle::Open)
    }
    fn chain_with_lifecycle(lifecycle: Lifecycle) -> (ForestProjection, Vec<VisitId>) {
        chain_length(lifecycle, 3)
    }
    fn chain_length(lifecycle: Lifecycle, length: usize) -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = (0..length)
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
                state: if i == length - 1 {
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
                current: Some(ids[length - 2]),
            },
            vec![TreeState {
                lifecycle,
                root: TreeId(ids[0]),
                last_visit: ids[length - 2],
                name: None,
            }],
        )
        .unwrap();
        (forest, ids)
    }

    #[test]
    fn current_pill_fits_reserved_room_at_rest_and_open() {
        let (forest, _) = chain_length(Lifecycle::Open, 40);
        for width in [390., 800., 1240., 1440.] {
            let strip = super::strip(&forest, width);
            let room = strip["room"].as_f64().unwrap();
            assert!(room < width);
            for mode in ["rest", "open"] {
                let layout = &strip[mode];
                let pill = layout["nodes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["kind"] == "pill")
                    .unwrap();
                let x = pill["x"].as_f64().unwrap() + layout["pan"].as_f64().unwrap();
                let half = pill["w"].as_f64().unwrap() / 2.;
                assert!(x - half >= -0.001 && x + half <= room + 0.001);
            }
        }
        let empty = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None,
            },
            vec![],
        )
        .unwrap();
        let strip = super::strip(&empty, 800.);
        assert_eq!(strip["room"], 800.);
        assert_eq!(strip["pins"], json!([]));
    }

    #[test]
    fn loaded_frame_serializes_all_three_states() {
        let (forest, ids) = chain();
        let mut frame = ChromeFrame {
            forest,
            loaded: HashSet::new(),
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            files: vec![],
        };
        let value = super::frame(&frame, 800.);
        assert_eq!(value["visits"][ids[2].to_string()]["when"], "not loaded");
        assert_eq!(value["waiting"]["ids"], json!([ids[2].to_string()]));
        frame.loaded.extend(ids.iter().copied());
        let value = super::frame(&frame, 800.);
        assert_eq!(value["visits"][ids[2].to_string()]["loaded"], true);
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], true);
        assert_eq!(value["visits"][ids[2].to_string()]["when"], "loaded");
        assert_eq!(value["visits"][ids[0].to_string()]["loaded"], false);
        assert_eq!(value["visits"][ids[0].to_string()]["queued"], false);
        assert_ne!(value["visits"][ids[0].to_string()]["when"], "loaded");
    }

    #[test]
    fn tabs_list_only_open_trees_and_none_is_active_while_the_current_tree_is_settled() {
        use crate::browser::{Browser, Navigation, NavigationTarget, Visibility};
        let dir = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&dir.path().join("forest.db")).unwrap();
        let url = url::Url::parse("https://example.org/a").unwrap();
        let mut navigation = Navigation::default();
        browser
            .navigate(&mut navigation, url.clone(), NavigationTarget::Root)
            .unwrap();
        let ticket = navigation.pending_ticket().unwrap();
        navigation.started(ticket);
        browser
            .commit_load(&mut navigation, ticket, url, Visibility::Selected)
            .unwrap();
        let current = browser.tree_states().unwrap()[0].root;
        browser
            .add_roots(&[url::Url::parse("https://example.org/b").unwrap()])
            .unwrap();
        browser.settle_tree(current).unwrap();
        let forest = ForestProjection::from_snapshot(
            browser.snapshot().unwrap(),
            browser.tree_states().unwrap(),
        )
        .unwrap();
        let frame = ChromeFrame {
            loaded: HashSet::new(),
            forest,
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            files: Vec::new(),
        };
        let value = super::frame(&frame, 1240.);
        let tabs = value["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["active"], false);
        assert_ne!(tabs[0]["id"], current.root().to_string());
    }

    #[test]
    fn settled_pages_stay_in_the_visits_map_so_search_and_the_forest_can_open_them() {
        let (forest, ids) = chain_with_lifecycle(Lifecycle::Settled { since: 1 });
        let frame = ChromeFrame {
            loaded: HashSet::new(),
            forest,
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            files: Vec::new(),
        };
        let value = super::frame(&frame, 1240.);
        assert_eq!(value["tabs"], json!([]));
        assert_eq!(value["current"], ids[1].to_string());
        assert_eq!(value["visits"].as_object().unwrap().len(), 3);
        assert_eq!(value["forest"]["labels"]["heads"][0]["settled"], true);
        assert!(value["forest"]["labels"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|n| n["settled"] == true));
        assert_eq!(value["forest"]["labels"]["divider"]["detail"], "1 tree");
    }
    #[test]
    fn the_frame_carries_every_visit_and_both_strip_states() {
        let (forest, ids) = chain();
        let frame = ChromeFrame {
            loaded: HashSet::new(),
            forest,
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            files: Vec::new(),
        };
        let value = super::frame(&frame, 1240.);
        assert_eq!(value["current"], ids[1].to_string());
        assert_eq!(value["visits"].as_object().unwrap().len(), 3);
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], true);
        assert_eq!(
            value["visits"][ids[0].to_string()]["site"],
            format!("example.org · {}", ago(0))
        );
        assert_eq!(value["tabs"][0]["pending"], 1);
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
    fn files_are_listed_under_their_page_oldest_first() {
        let (forest, ids) = chain();
        let file = |id, visit, path: &str, size, missing| PageFile {
            download: Download {
                id,
                visit,
                path: path.into(),
                size,
                source: None,
                saved_at: 0,
            },
            missing,
        };
        let frame = ChromeFrame {
            loaded: HashSet::new(),
            forest,
            capture_enabled: true,
            theme: ThemeMode::Dark,
            suggestions: StartSuggestions::default(),
            files: vec![
                file(
                    1,
                    ids[1],
                    "/home/me/Saved files/report.pdf",
                    1_234_567,
                    false,
                ),
                file(2, ids[0], "/home/me/Saved files/data", 0, true),
                file(3, ids[1], "/home/me/Saved files/report (1).pdf", 812, false),
            ],
        };
        let value = super::frame(&frame, 1240.);
        let files = value["files"].as_object().unwrap();
        assert_eq!(files.len(), 2);
        assert_eq!(
            files[&ids[1].to_string()],
            json!([
                { "id": 1, "name": "report.pdf", "detail": format!("1.2 MB · {}", ago(0)),
                  "folder": "Saved files", "missing": false },
                { "id": 3, "name": "report (1).pdf", "detail": format!("812 B · {}", ago(0)),
                  "folder": "Saved files", "missing": false },
            ])
        );
        assert_eq!(
            files[&ids[0].to_string()],
            json!([{ "id": 2, "name": "data", "detail": "Moved or deleted",
                     "folder": "Saved files", "missing": true }])
        );
    }

    #[test]
    fn sizes_read_like_a_file_manager() {
        for (bytes, text) in [
            (0, "0 B"),
            (999, "999 B"),
            (1000, "1.0 kB"),
            (9_949, "9.9 kB"),
            (9_950, "10 kB"),
            (200_000, "200 kB"),
            (999_499, "999 kB"),
            (999_500, "1.0 MB"),
            (1_234_567, "1.2 MB"),
            (48_000_000_000, "48 GB"),
            (u64::MAX, "18446744 TB"),
        ] {
            assert_eq!(size_text(bytes), text, "{bytes}");
        }
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
