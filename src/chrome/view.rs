//! What the chrome view draws, as JSON. Rust decides every position and every string; the
//! HTML view only paints them and reports input back.

use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use serde_json::{json, Value};

use super::{
    graph::{self, EdgeKind, StripItem},
    palette::{Group, PaletteItem},
    pins, tree_view,
    waiting::{self, WaitState, WaitingOrder},
    ChromeFrame, PageFile,
};
use crate::{
    browser::{Capture, CaptureKind, ThemeMode, Visit, VisitId, VisitPreview, VisitState},
    forest::ForestProjection,
};

/// Room at the end of the current-page pill for the Saved text dot.
const ACCESSORY: f64 = 22.;

#[derive(Clone, Copy, Default)]
pub(crate) struct Surfaces {
    pub start: bool,
    pub palette: Option<VisitId>,
}

pub(crate) fn frame(
    frame: &ChromeFrame,
    suggested: &[crate::suggest::Suggestion],
    viewport: (f64, f64),
    workbench: &tree_view::Workbench,
    surfaces: Surfaces,
) -> Value {
    let width = viewport.0;
    let forest = &frame.forest;
    let current = forest.current();
    let active = current.and_then(|id| forest.tree_of(id));
    let tabs: Vec<Value> = forest
        .tabs()
        .iter()
        .map(|tree| {
            json!({
                "id": tree.id.root().to_string(),
                "title": tree.title,
                "subtitle": super::tabs::subtitle(forest, tree),
                "name": format!("{}, {}{}{}{}", tree.title, plural(tree.page_count, "page"), if tree.pending_count > 0 { format!(", {} waiting", tree.pending_count) } else { String::new() }, if tree.place.pinned() { ", pinned" } else { "" }, if frame.live.contains(&tree.last_visit) { ", still open" } else { "" }),
                "pages": tree.page_count,
                "live": frame.live.contains(&tree.last_visit),
                "pending": tree.pending_count,
                "active": Some(tree.id) == active,
                "pinned": tree.place.pinned(),
                "menu": super::tree_card::menu(tree),
            })
        })
        .collect();
    let start = surfaces
        .start
        .then(|| super::start::value(frame, suggested));
    let mut visible = HashSet::new();
    for tree in forest.trees() {
        if Some(tree.id) == active {
            let mut stack = vec![tree.id.root()];
            while let Some(id) = stack.pop() {
                visible.insert(id);
                stack.extend(forest.children(id).iter().copied());
            }
        }
    }
    visible.extend(surfaces.palette);
    visible.extend(forest.tabs().iter().map(|t| t.id.root()));
    if let Some(start) = &start {
        for mini in start["shelf"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| &p["mini"])
            .chain(
                start["suggested"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|p| &p["glyph"]["mini"]),
            )
        {
            for node in mini["nodes"].as_array().into_iter().flatten() {
                if let Some(id) = node["id"].as_str().and_then(|s| s.parse::<VisitId>().ok()) {
                    visible.insert(id);
                }
            }
        }
    }
    let mut visits = serde_json::Map::new();
    for id in visible {
        if let Some(visit) = forest.visit(id) {
            visits.insert(
                id.to_string(),
                visit_value(forest, visit, &frame.loaded, &frame.live),
            );
        }
    }
    let waiting_pages = waiting::pages(forest);
    let waiting_name = waiting_pages.first().map_or_else(
        || "No waiting pages".into(),
        |next| {
            let visit = &visits[&next.id.to_string()];
            let label = visit["label"].as_str().unwrap();
            let state = visit["a11yState"]
                .as_str()
                .unwrap()
                .replace(", waiting", "");
            format!("{} waiting, next {label}{state}", waiting_pages.len())
        },
    );
    // A new tree has no root until its page commits; its tab is drawn from the address.
    let new_tree = match &frame.pending {
        Some(crate::browser::PendingTarget::NewTree(url)) => Some(json!({
            "url": url.as_str(),
            "title": url.host_str().map_or_else(|| url.to_string(), |host| host.trim_start_matches("www.").to_owned()),
        })),
        _ => None,
    };
    json!({
        "current": current.map(|id| id.to_string()),
        "newTree": new_tree,
        "tabs": tabs,
        "visits": visits,
        "strip": strip_for_frame(frame, width),
        "treeView": tree_view_value(forest, workbench, viewport),
        "waiting": { "name": waiting_name, "order": WaitingOrder::CHOSEN.label(), "ids": waiting_pages.iter().map(|v| v.id.to_string()).collect::<Vec<_>>() },
        "files": files_value(&frame.files),
        "captureEnabled": frame.capture_enabled,
        "mail": mail_value(&frame.mail),
        "treesHelp": frame.trees_help,
        "theme": match frame.theme {
            ThemeMode::System => "system",
            ThemeMode::Light => "light",
            ThemeMode::Dark => "dark",
        },
        "start": start,
    })
}

pub(crate) fn tree_view_value(
    forest: &ForestProjection,
    state: &tree_view::Workbench,
    viewport: (f64, f64),
) -> Value {
    let Some(view) = tree_view::workbench(forest, state) else {
        return Value::Null;
    };
    let current_tree = forest.current().and_then(|id| forest.tree_of(id));
    let Some(tree) = forest.trees().iter().find(|t| Some(t.id) == current_tree) else {
        return Value::Null;
    };
    json!({
        "title": tree.title,
        "query": state.query, "waitingOnly": state.waiting_only,
        "overflows": view.width > viewport.0 || view.height > viewport.1 - (super::CHROME_HEIGHT - graph::REST_HEIGHT) - 52. - 42. - 48.,
        "detail": format!("{} · {} waiting", plural(tree.page_count, "page"), tree.pending_count),
        "width": view.width, "height": view.height,
        "tiles": view.tiles.iter().map(|t| json!({"id": t.id.to_string(), "x": t.rect.x, "y": t.rect.y, "w": t.rect.w, "s": t.rect.s, "path": t.on_path})).collect::<Vec<_>>(),
        "edges": view.edges.iter().map(|e| json!({"from": e.from.to_string(), "to": e.to.to_string(), "kind": match e.kind { EdgeKind::Path => "path", EdgeKind::Pending => "pending", EdgeKind::Plain => "plain" }, "c": e.curve})).collect::<Vec<_>>(),
        "rows": view.rows.iter().map(|r| {
            let visit = forest.visit(r.id).unwrap();
            let settled = visit.branch_settled_at.is_some();
            let folded = state.query.is_empty() && !state.waiting_only && state.folded(forest, r.id);
            let short_name = graph::short_name(forest, r.id);
            let siblings = forest.siblings(r.id);
            let related = siblings.iter().filter(|&&id| id != r.id).map(|&id| graph::short_name(forest, id))
                .max_by_key(|other| short_name.chars().zip(other.chars()).take_while(|(a, b)| a == b).count());
            let tile_title = related.map_or_else(|| short_name.clone(), |other| {
                let distinct = graph::distinct_name(&short_name, &other);
                if distinct != short_name { return distinct; }
                let common = short_name.chars().zip(other.chars()).take_while(|(a, b)| a == b).filter(|(c, _)| c.is_whitespace()).count();
                if common == 0 { return short_name.clone(); }
                let words: Vec<_> = short_name.split_whitespace().collect();
                if words.len() <= 3 { return short_name.clone(); }
                format!("…{}", words[words.len()-3..].join(" "))
            });
            let branch = !forest.children(r.id).is_empty();
            let mut waiting = 0;
            let mut stack = if folded { vec![r.id] } else { Vec::new() };
            while let Some(id) = stack.pop() {
                let v = forest.visit(id).unwrap();
                waiting += usize::from(v.state == VisitState::Queued && v.dismissed_at.is_none() && (settled || !forest.is_settled(id)));
                stack.extend(forest.children(id));
            }
            json!({"id": r.id.to_string(), "depth": r.depth, "level": forest.path(r.id).len(), "path": r.on_path,
                "title": if folded && settled { format!("{} pages put away", forest.descendant_count(r.id).unwrap_or(0) + 1) } else { short_name },
                "tileTitle": if folded && settled { format!("{} pages put away", forest.descendant_count(r.id).unwrap_or(0) + 1) } else { tile_title },
                "branch": branch, "folded": folded,
                "summary": if folded && settled { format!(" · {waiting} waiting") }
                    else if folded && branch { format!("{} pages · {} waiting", forest.descendant_count(r.id).unwrap_or(0) + 1, waiting) }
                    else if settled { "put away".to_owned() } else { String::new() }})
        }).collect::<Vec<_>>(),
    })
}

pub(crate) fn strip_for_frame(frame: &ChromeFrame, width: f64) -> Value {
    let mut strip = strip(&frame.forest, &frame.mail, width);
    let target = |id: Option<VisitId>| {
        id.and_then(|id| frame.forest.visit(id))
            .map(|v| json!({ "id": v.id.to_string(), "label": v.label() }))
    };
    let current = frame.forest.current();
    strip["back"] = json!(target(current.and(frame.back)));
    strip["forward"] = json!(target(current.and(frame.forward)));
    let state = |id: &str| {
        let id: VisitId = id.parse().unwrap();
        format!(
            "{}{}",
            if frame.loaded.contains(&id) && frame.forest.is_waiting(id) {
                ", loaded"
            } else {
                ""
            },
            if frame.live.contains(&id) {
                ", still open"
            } else {
                ""
            }
        )
    };
    for node in strip["nodes"].as_array_mut().unwrap() {
        node["name"] = json!(format!(
            "{}{}",
            node["name"].as_str().unwrap(),
            state(node["page"].as_str().unwrap())
        ));
        for row in node["rows"].as_array_mut().unwrap() {
            row["name"] = json!(format!(
                "{}{}",
                row["name"].as_str().unwrap(),
                state(row["id"].as_str().unwrap())
            ));
        }
    }
    strip
}

fn branch_name(label: &str, pages: usize, waiting: usize, queued: bool) -> String {
    let mut name = label.to_owned();
    if pages > 0 {
        name.push_str(&format!(
            ", {pages} branch {}",
            if pages == 1 { "page" } else { "pages" }
        ));
    }
    if waiting > 0 {
        name.push_str(&format!(", {waiting} waiting"));
    } else if queued {
        name.push_str(", waiting");
    }
    name
}

/// The current path folded to fit the room left by the pinned controls.
pub(crate) fn strip(forest: &ForestProjection, mail: &crate::mail::Mail, width: f64) -> Value {
    let pins = pins::pins(forest, mail, width);
    let placed: Vec<_> = pins
        .placed
        .iter()
        .map(|(pin, r)| json!({ "kind": pin.kind(), "x": r.x, "y": r.y, "w": r.w, "s": r.s }))
        .collect();
    // In a narrow window the tree keeps the room; the keys and mouse buttons still work.
    let left = if pins.room - pins::NAV >= pins::NAV_MIN_ROOM {
        pins::NAV
    } else {
        0.
    };
    let room = (pins.room - left).max(0.);
    let Some(current) = forest.current() else {
        return json!({ "left": left, "room": room, "pins": placed, "height": graph::REST_HEIGHT, "nodes": [], "edges": [], "trunk": [], "width": 0, "pan": 0 });
    };
    let layout = graph::strip_layout(
        forest,
        current,
        room,
        forest.visit(current).map_or(120., pill_width),
    );
    let path = forest.path(current);
    let nodes: Vec<_> = layout.iter().map(|node| {
        let ids = node.item.ids();
        let id = ids.last().copied().unwrap_or(current);
        let v = forest.visit(id).unwrap();
        let (kind, label, title, meta, heading, more) = match &node.item {
            StripItem::Root(_) | StripItem::Page(_) | StripItem::Parent(_) | StripItem::Current(_) => {
                let kind = match node.item { StripItem::Root(_) => "root", StripItem::Page(_) => "page", StripItem::Parent(_) => "parent", _ => "pill" };
                (kind, if v.id != current && forest.visit(current).unwrap().parent == Some(id) { graph::parent_name(forest, id, current) } else { graph::short_name(forest, id) }, v.label().to_owned(), format!("{} · {} after", site_line(v), plural(forest.descendant_count(id).unwrap_or(0), "page")),
                    if node.total_rows == 0 { String::new() } else { format!("Other branches from here · {}", node.total_rows) },
                    if node.total_rows > node.rows.len() { format!("{} more", node.total_rows - node.rows.len()) } else { String::new() })
            }
            StripItem::Run { ids, host } => {
                let first = forest.visit(ids[0]).unwrap();
                let before = first.parent.and_then(|id| forest.visit(id)).map_or("Root", Visit::label);
                let after = path.iter().position(|id| Some(id) == ids.last()).and_then(|i| path.get(i+1)).and_then(|id| forest.visit(*id)).map_or("Current page", Visit::label);
                ("run", ids.len().to_string(), format!("{} on {host}", plural(ids.len(), "page")), format!("Between {before} and {after}"), String::new(),
                    if node.total_rows > node.rows.len() { format!("{} more, in the tree view", node.total_rows - node.rows.len()) } else { String::new() })
            }
            StripItem::Earlier { ids } => ("earlier", format!("··· {} earlier", ids.len()), format!("{} earlier pages", ids.len()),
                format!("From {} to {}", forest.visit(ids[0]).unwrap().label(), v.label()), "Most recent first".into(),
                if node.total_rows > node.rows.len() { format!("{} more, in the tree view", node.total_rows - node.rows.len()) } else { String::new() }),
            StripItem::After { waiting, read } => ("after", String::new(), v.label().to_owned(), site_line(v),
                if waiting.is_empty() { String::new() } else { format!("Waiting from this page · {}", waiting.len()) }, nonzero_parts(&[(waiting.len().saturating_sub(node.rows.len()), "more waiting"), (read.len(), "read")])),
        };
        let rows: Vec<_> = node.rows.iter().map(|id| {
            let counts = if matches!(node.item, StripItem::Run { .. } | StripItem::Earlier { .. }) {
                graph::branch_counts(forest, &graph::branch_heads(forest, &[*id], &path))
            } else { graph::branch_counts(forest, &[*id]) };
            json!({ "id": id.to_string(), "name": branch_name(forest.visit(*id).unwrap().label(), counts.pages, counts.waiting, forest.is_waiting(*id)), "label": graph::short_name(forest, *id), "pages": counts.pages, "waiting": counts.waiting,
                "detail": if matches!(node.item, StripItem::Run { .. } | StripItem::Earlier { .. }) && counts.pages == 0 {
                    ago(forest.visit(*id).unwrap().opened_at.unwrap_or(forest.visit(*id).unwrap().created_at))
                } else if forest.visit(*id).unwrap().branch_settled_at.is_some() { format!("{} pages put away", forest.descendant_count(*id).unwrap_or(0)+1) } else { nonzero_parts(&[(counts.pages, if counts.pages == 1 { "page" } else { "pages" }), (counts.waiting, "waiting")]) } })
        }).collect();
        let (waiting, read) = match &node.item { StripItem::After { waiting, read } => (waiting.len(), read.len()), _ => (0, 0) };
        let page_head = matches!(node.item, StripItem::Root(_) | StripItem::Page(_) | StripItem::Parent(_) | StripItem::Current(_));
        let mut actions = Vec::new();
        if page_head {
            actions.push(json!({ "kind": if id == current { "reload" } else { "open" }, "label": if id == current { "Reload" } else { "Open" } }));
            actions.push(json!({ "kind": "branch", "label": "New branch here" }));
            if id == current {
                if let Some(source) = &v.source {
                    if v.parent.is_some() { actions.push(json!({ "kind": "source", "label": "Back to source" })); }
                    if source.citation_url.is_some() { actions.push(json!({ "kind": "comment", "label": "Open comment" })); }
                }
            }
        }
        json!({
            "preview": page_head, "actions": actions,
            "moreTarget": if let StripItem::Earlier { ids } = &node.item { ids.first().copied().unwrap_or(current).to_string() } else { current.to_string() },
            "id": if kind == "after" { format!("after-{current}") } else { id.to_string() },
            "page": id.to_string(), "ids": ids.iter().map(ToString::to_string).collect::<Vec<_>>(),
            "named": kind == "root" && forest.visit(current).unwrap().parent == Some(id),
            "kind": kind, "name": if kind == "after" { format!("After {title}{}{}", if waiting > 0 { format!(", {} waiting {}", waiting, if waiting == 1 { "page" } else { "pages" }) } else { String::new() }, if read > 0 { format!(", {read} read {}", if read == 1 { "page" } else { "pages" }) } else { String::new() }) } else { branch_name(&title, node.branches.pages, node.branches.waiting, forest.is_waiting(id)) }, "label": label, "title": title, "meta": meta, "heading": heading, "more": more, "rows": rows,
            "branches": { "pages": node.branches.pages, "waiting": node.branches.waiting },
            "waiting": waiting, "read": read,
            "x": node.rect.x, "y": node.rect.y, "w": node.rect.w, "s": node.rect.s,
        })
    }).collect();
    let edges: Vec<_> = nodes.windows(2).zip(layout.windows(2)).map(|(nodes, rects)| {
        let a = rects[0].rect; let b = rects[1].rect;
        let x = a.x+a.w/2.; let end = b.x-b.w/2.;
        json!({"from": nodes[0]["id"], "to": nodes[1]["id"], "kind": if nodes[1]["kind"] == "after" { "pending" } else { "path" },
            "c": [[x,a.y], [x+5.,a.y], [end-5.,b.y], [end,b.y]]})
    }).collect();
    json!({ "left": left, "room": room, "pins": placed, "height": graph::REST_HEIGHT, "width": room, "pan": 0,
        "metrics": { "left": graph::ITEM_LEFT, "right": graph::ITEM_RIGHT, "border": graph::ITEM_BORDER, "icon": graph::ITEM_ICON, "parentGap": graph::PARENT_GAP, "runGap": graph::RUN_GAP },
        "trunk": nodes.iter().map(|n| n["id"].clone()).collect::<Vec<_>>(), "nodes": nodes, "edges": edges })
}

fn nonzero_parts(parts: &[(usize, &str)]) -> String {
    parts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, label)| format!("{n} {label}"))
        .collect::<Vec<_>>()
        .join(" · ")
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

pub(super) fn visit_value(
    forest: &ForestProjection,
    visit: &Visit,
    loaded: &HashSet<VisitId>,
    live: &HashSet<VisitId>,
) -> Value {
    let wait = if forest.is_settled(visit.id) {
        None
    } else {
        WaitState::of(visit, loaded)
    };
    let a11y_state = format!(
        "{}{}{}",
        if forest.is_waiting(visit.id) {
            ", waiting"
        } else {
            ""
        },
        if wait == Some(WaitState::Loaded) {
            ", loaded"
        } else {
            ""
        },
        if live.contains(&visit.id) {
            ", still open"
        } else {
            ""
        }
    );
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
    let detail = visit
        .source
        .as_ref()
        .map(|source| source.link_text.as_str())
        .filter(|text| !text.is_empty())
        .unwrap_or(if wait == Some(WaitState::Loaded) {
            "loaded"
        } else {
            ""
        });
    json!({
        "id": visit.id.to_string(),
        "parent": visit.parent.map(|id| id.to_string()),
        "url": visit.url.as_str(),
        "label": visit.label(),
        "shortName": graph::short_name(forest, visit.id),
        "name": format!("{}{}", visit.label(), a11y_state),
        "a11yState": a11y_state,
        "site": site_line(visit),
        "queued": forest.is_waiting(visit.id),
        "dismissed": visit.dismissed_at.is_some(),
        "stub": if visit.branch_settled_at.is_some() { format!("{} pages put away", forest.descendant_count(visit.id).unwrap_or(0)+1) } else { String::new() },
        "nodeMenu": node_menu(forest, visit),
        "loaded": wait == Some(WaitState::Loaded),
        "live": live.contains(&visit.id),
        "detail": detail,
        "when": match wait { Some(WaitState::NotLoaded) => String::new(), Some(WaitState::Loaded) => "loaded".into(), None => ago(visit.opened_at.unwrap_or(visit.created_at)) },
        "source": source,
    })
}

/// Waiting pages the Ctrl+Tab list shows before the read ones; the rest are counted.
const SWITCHER_WAITING: usize = 5;
/// Read pages the Ctrl+Tab list shows.
pub(crate) const SWITCHER_READ: usize = 8;

/// The Ctrl+Tab list: the current tree's next waiting pages, then `recent`, its read pages
/// most recently shown first. A quick Ctrl+Tab opens the first row.
pub(crate) fn switcher_value(frame: &ChromeFrame, recent: &[VisitId]) -> Value {
    let forest = &frame.forest;
    let waiting = waiting::pages(forest);
    let row = |visit: &Visit| visit_value(forest, visit, &frame.loaded, &frame.live);
    json!({
        "waiting": waiting.iter().take(SWITCHER_WAITING).map(|v| row(v)).collect::<Vec<_>>(),
        "moreWaiting": waiting.len().saturating_sub(SWITCHER_WAITING),
        "read": recent.iter().filter_map(|id| forest.visit(*id)).take(SWITCHER_READ).map(row).collect::<Vec<_>>(),
    })
}

fn node_menu(forest: &ForestProjection, visit: &Visit) -> Vec<Value> {
    let row = |kind: &str, label: String| json!({ "kind": kind, "label": label });
    let mut rows = vec![
        row("open", "Open".into()),
        row("new-tree", "Open in a new tree".into()),
        row(
            if visit.state == VisitState::Queued {
                "branch-queued"
            } else {
                "branch"
            },
            "New branch here".into(),
        ),
        row("copy", "Copy address".into()),
    ];
    let waiting = forest.pending_descendant_count(visit.id).unwrap_or(0)
        + usize::from(forest.is_waiting(visit.id));
    if waiting > 0 {
        rows.push(row(
            "dismiss",
            if forest.is_waiting(visit.id) {
                "Dismiss".into()
            } else {
                format!("Dismiss {waiting} waiting")
            },
        ));
    }
    if visit.parent.is_some() {
        if !forest.is_settled(visit.id) {
            rows.push(row("settle-branch", "Settle branch".into()));
        }
        if visit.state == VisitState::Opened || forest.opened_descendant_count(visit.id) > 0 {
            rows.push(row("move-to-tree", "Move to its own tree".into()));
        }
    }
    rows
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
                    let pending = if forest.is_waiting(*id) { " · waiting" } else if visit.dismissed_at.is_some() { " · skipped" } else { "" };
                    let icon = json!({ "url": visit.url.as_str(), "opened": visit.state == VisitState::Opened });
                    ("page", visit.label().to_owned(), format!("{tree}{pending}"), icon)
                }
                PaletteItem::Saved { title, excerpt, .. } => {
                    let excerpt = excerpt.split_whitespace().collect::<Vec<_>>().join(" ");
                    ("saved", title.clone(), excerpt.chars().take(140).collect(), json!("search"))
                }
                PaletteItem::Suggestion(s) => {
                    let chip = super::start::chip(forest, s);
                    let url = match s {
                        crate::suggest::Suggestion::Page { url, .. } | crate::suggest::Suggestion::Path { first: url, .. } => url,
                        crate::suggest::Suggestion::Tree { tree, .. } => &forest.visit(tree.root()).unwrap().url,
                    };
                    ("suggestion", chip["title"].as_str().unwrap().to_owned(), s.why(), json!({"url":url.as_str(),"opened":true}))
                }
                PaletteItem::Imported(page) => {
                    let host = short_host(&page.url).to_owned();
                    let title = if page.title.trim().is_empty() { host.clone() } else { page.title.clone() };
                    let detail = if page.visits > 0 { format!("{host} · {}", plural(page.visits, "visit")) } else { host };
                    ("imported", title, detail, json!({ "url": page.url.as_str(), "opened": true }))
                }
                PaletteItem::Bookmark(b) => ("bookmark", super::start::bookmark_label(b).to_owned(), b.folder.clone().unwrap_or_else(|| "Bookmarks".into()), json!({"url": b.url.as_str(), "opened":true})),
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
pub(super) fn text_width(text: &str, size: f64, em: f64) -> f64 {
    text.chars().count() as f64 * size * em
}

pub(crate) fn site_line(visit: &Visit) -> String {
    let host = short_host(&visit.url);
    match visit.state {
        VisitState::Queued => format!("{host} · waiting"),
        VisitState::Opened => format!(
            "{host} · {}",
            ago(visit.opened_at.unwrap_or(visit.created_at))
        ),
    }
}

pub(super) fn short_host(url: &url::Url) -> &str {
    let host = url.host_str().unwrap_or_default();
    host.strip_prefix("www.").unwrap_or(host)
}

pub(super) fn plural(count: usize, noun: &str) -> String {
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
    use crate::browser::Spot;
    #[test]
    fn branch_names_omit_zero_counts_and_repeat_no_state() {
        assert_eq!(super::branch_name("Leaf", 0, 0, false), "Leaf");
        assert_eq!(
            super::branch_name("Fork", 1, 0, false),
            "Fork, 1 branch page"
        );
        assert_eq!(
            super::branch_name("Fork", 2, 1, true),
            "Fork, 2 branch pages, 1 waiting"
        );
        assert_eq!(super::branch_name("Leaf", 0, 0, true), "Leaf, waiting");
    }

    use super::*;
    use crate::browser::{Download, Snapshot, TreeId, TreeState};

    fn chain() -> (ForestProjection, Vec<VisitId>) {
        chain_with_lifecycle(crate::browser::Spot::Tab {
            pinned: false,
            position: 0,
        })
    }
    fn chain_with_lifecycle(spot: Spot) -> (ForestProjection, Vec<VisitId>) {
        chain_length(spot, 3)
    }
    fn chain_length(spot: Spot, length: usize) -> (ForestProjection, Vec<VisitId>) {
        let ids: Vec<VisitId> = (0..length)
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        let visits = ids
            .iter()
            .enumerate()
            .map(|(i, &id)| Visit {
                dismissed_at: None,
                branch_settled_at: None,
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
                last_event_at: None,
                spot,
                root: TreeId(ids[0]),
                last_visit: ids[length - 2],
                name: None,
            }],
        )
        .unwrap();
        (forest, ids)
    }

    #[test]
    fn back_and_forward_name_their_pages_and_the_tree_starts_after_them() {
        let (forest, ids) = chain_length(
            crate::browser::Spot::Tab {
                pinned: false,
                position: 0,
            },
            4,
        );
        let frame = |forest: ForestProjection, back: Option<VisitId>, forward: Option<VisitId>| {
            ChromeFrame {
                shown: Default::default(),
                mail: crate::mail::Mail::default(),
                loaded: HashSet::new(),
                live: HashSet::new(),
                forest,
                capture_enabled: true,
                trees_help: true,
                theme: ThemeMode::Dark,
                declined: Default::default(),
                bookmarks: vec![],
                back,
                forward,
                pending: None,
                files: Vec::new(),
            }
        };
        let mut chain = frame(forest, Some(ids[1]), Some(ids[3]));
        let strip = strip_for_frame(&chain, 1240.);
        assert_eq!(
            strip["back"],
            json!({ "id": ids[1].to_string(), "label": "Page 1" })
        );
        assert_eq!(
            strip["forward"],
            json!({ "id": ids[3].to_string(), "label": "Page 3" })
        );
        // Back can return through WebKit's history to a page that is not the parent.
        chain.back = Some(ids[0]);
        assert_eq!(strip_for_frame(&chain, 1240.)["back"]["label"], "Page 0");
        assert_eq!(strip["left"], pins::NAV);
        let pins = pins::pins(
            &chain_length(
                crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
                4,
            )
            .0,
            &crate::mail::Mail::default(),
            1240.,
        );
        assert_eq!(strip["room"], pins.room - pins::NAV);

        let (root, _) = chain_length(
            crate::browser::Spot::Tab {
                pinned: false,
                position: 0,
            },
            2,
        );
        let strip = strip_for_frame(&frame(root, None, None), 1240.);
        assert!(strip["back"].is_null(), "the root has no page before it");
        assert!(strip["forward"].is_null());

        let narrow = strip_for_frame(
            &frame(
                chain_length(
                    crate::browser::Spot::Tab {
                        pinned: false,
                        position: 0,
                    },
                    4,
                )
                .0,
                None,
                None,
            ),
            390.,
        );
        assert_eq!(
            narrow["left"], 0.,
            "a narrow window leaves the room to the tree"
        );
        let pins = pins::pins(
            &chain_length(
                crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
                4,
            )
            .0,
            &crate::mail::Mail::default(),
            390.,
        );
        assert_eq!(narrow["room"], pins.room);

        let empty = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None,
            },
            vec![],
        )
        .unwrap();
        let strip = strip_for_frame(&frame(empty, Some(ids[1]), Some(ids[3])), 1240.);
        assert!(strip["back"].is_null() && strip["forward"].is_null());
    }

    #[test]
    fn fully_listed_waiting_children_have_no_empty_more_row() {
        let (forest, _) = chain();
        let value = strip(&forest, &crate::mail::Mail::default(), 1240.);
        let after = value["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "after")
            .unwrap();
        assert_eq!(after["rows"].as_array().unwrap().len(), 1);
        assert_eq!(after["more"], "");
        assert_eq!(nonzero_parts(&[(1, "page"), (0, "waiting")]), "1 page");
        assert_eq!(nonzero_parts(&[(0, "pages"), (2, "waiting")]), "2 waiting");
    }

    #[test]
    fn the_frame_carries_the_current_tree_view() {
        let (forest, _) = chain();
        let view = tree_view_value(&forest, &tree_view::Workbench::default(), (820., 800.));
        assert_eq!(view["detail"], "3 pages · 1 waiting");
        assert_eq!(view["tiles"].as_array().unwrap().len(), 3);
        let fits = view["height"].as_f64().unwrap() + 42. + 52. + 42. + 48.;
        assert_eq!(
            tree_view_value(&forest, &tree_view::Workbench::default(), (820., fits))["overflows"],
            false
        );
        assert_eq!(
            tree_view_value(&forest, &tree_view::Workbench::default(), (820., fits - 1.))
                ["overflows"],
            true
        );
        let empty = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None,
            },
            vec![],
        )
        .unwrap();
        assert!(tree_view_value(&empty, &tree_view::Workbench::default(), (820., 800.)).is_null());
    }

    #[test]
    fn current_pill_fits_reserved_room() {
        let (forest, _) = chain_length(
            crate::browser::Spot::Tab {
                pinned: false,
                position: 0,
            },
            40,
        );
        for width in [390., 800., 1240., 1440.] {
            let strip = super::strip(&forest, &crate::mail::Mail::default(), width);
            let room = strip["room"].as_f64().unwrap();
            assert!(room < width);
            let layout = &strip;
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
        let empty = ForestProjection::from_snapshot(
            Snapshot {
                visits: vec![],
                current: None,
            },
            vec![],
        )
        .unwrap();
        let strip = super::strip(&empty, &crate::mail::Mail::default(), 800.);
        assert_eq!(strip["room"], 800. - pins::NAV);
        assert_eq!(strip["left"], pins::NAV);
        assert_eq!(strip["pins"], json!([]));
    }

    #[test]
    fn switcher_lists_five_waiting_pages_then_the_read_ones() {
        use crate::browser::{Snapshot, TreeState, Visit, VisitState};
        let ids: Vec<VisitId> = (0..8)
            .map(|_| ulid::Ulid::new().to_string().parse().unwrap())
            .collect();
        // 0 is the root and current page, 1 a read child, 2..8 waiting children.
        let visits = (0..8)
            .map(|i| {
                let opened = i < 2;
                Visit {
                    dismissed_at: None,
                    branch_settled_at: None,
                    id: ids[i],
                    parent: (i > 0).then(|| ids[0]),
                    url: url::Url::parse(&format!("https://example.org/{i}")).unwrap(),
                    title: format!("Page {i}"),
                    created_at: i as i64,
                    opened_at: opened.then_some(i as i64),
                    state: if opened {
                        VisitState::Opened
                    } else {
                        VisitState::Queued
                    },
                    source: None,
                }
            })
            .collect();
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(ids[0]),
            },
            vec![TreeState {
                last_event_at: None,
                root: TreeId(ids[0]),
                last_visit: ids[0],
                name: None,
                spot: Spot::Tab {
                    pinned: false,
                    position: 0,
                },
            }],
        )
        .unwrap();
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            forest,
            loaded: Default::default(),
            live: [ids[1]].into_iter().collect(),
            capture_enabled: true,
            trees_help: false,
            theme: ThemeMode::Light,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let value = super::switcher_value(&frame, &[ids[1]]);
        let listed = |key: &str| {
            value[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let id = |i: usize| ids[i].to_string();
        assert_eq!(listed("waiting"), [id(7), id(6), id(5), id(4), id(3)]);
        assert_eq!(value["moreWaiting"], 1);
        assert_eq!(listed("read"), [id(1)]);
        assert_eq!(value["read"][0]["live"], true);
    }

    #[test]
    fn closed_surfaces_omit_history_and_open_surfaces_include_their_pages() {
        let dir = tempfile::tempdir().unwrap();
        let mut browser = crate::browser::Browser::open(&dir.path().join("db")).unwrap();
        let roots = browser
            .add_roots(&[
                url::Url::parse("https://active.example").unwrap(),
                url::Url::parse("https://settled.example").unwrap(),
            ])
            .unwrap();
        browser
            .place_tree(TreeId(roots[1]), crate::browser::Move::History)
            .unwrap();
        browser
            .place_tree(
                TreeId(roots[1]),
                crate::browser::Move::Shelf(crate::browser::Slot::End),
            )
            .unwrap();
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            forest: ForestProjection::from_snapshot(
                browser.snapshot().unwrap(),
                browser.tree_states().unwrap(),
            )
            .unwrap(),
            loaded: Default::default(),
            live: Default::default(),
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Light,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let closed = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces::default(),
        );
        assert!(closed["forest"].is_null());
        assert!(closed["start"].is_null());
        assert!(closed["visits"][roots[1].to_string()].is_null());
        let scoped = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                palette: Some(roots[1]),
                ..Surfaces::default()
            },
        );
        assert!(scoped["visits"][roots[1].to_string()].is_object());
        let start = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(start["start"]["shelf"].as_array().unwrap().len(), 1);
        assert!(start["visits"][roots[1].to_string()].is_object());
    }

    #[test]
    fn loaded_frame_serializes_all_three_states() {
        let (forest, ids) = chain();
        let mut frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            forest,
            loaded: HashSet::new(),
            live: HashSet::new(),
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let value = super::frame(
            &frame,
            &[],
            (800., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(value["visits"][ids[2].to_string()]["when"], "");
        assert_eq!(value["waiting"]["ids"], json!([ids[2].to_string()]));
        assert_eq!(
            value["visits"][ids[2].to_string()]["a11yState"],
            ", waiting"
        );
        frame.live.insert(ids[0]);
        frame.loaded.extend(ids.iter().copied());
        let value = super::frame(
            &frame,
            &[],
            (800., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(
            value["visits"][ids[2].to_string()]["a11yState"],
            ", waiting, loaded"
        );
        assert_eq!(
            value["visits"][ids[0].to_string()]["a11yState"],
            ", still open"
        );
        assert!(!value.to_string().to_lowercase().contains("memory"));
        assert_eq!(value["strip"], super::strip_for_frame(&frame, 800.));
        assert!(value["strip"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"].as_str().unwrap().contains("still open")));
        assert_eq!(value["visits"][ids[2].to_string()]["loaded"], true);
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], true);
        assert_eq!(value["visits"][ids[2].to_string()]["when"], "loaded");
        assert_eq!(value["visits"][ids[0].to_string()]["loaded"], false);
        assert_eq!(value["visits"][ids[0].to_string()]["queued"], false);
        assert_ne!(value["visits"][ids[0].to_string()]["when"], "loaded");
    }

    #[test]
    fn waiting_detail_uses_link_text_then_loaded_or_nothing() {
        let (forest, ids) = chain();
        let mut visit = forest.visit(ids[2]).unwrap().clone();
        let loaded = HashSet::from([visit.id]);
        let empty = HashSet::new();
        assert_eq!(visit_value(&forest, &visit, &empty, &empty)["detail"], "");
        assert_eq!(
            visit_value(&forest, &visit, &loaded, &empty)["detail"],
            "loaded"
        );
        visit.source = Some(crate::browser::SourceContext {
            page_url: visit.url.clone(),
            citation_url: None,
            link_text: "Why this link matters".into(),
            excerpt: String::new(),
        });
        for state in [&empty, &loaded] {
            assert_eq!(
                visit_value(&forest, &visit, state, &empty)["detail"],
                "Why this link matters"
            );
        }
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
        browser
            .place_tree(current, crate::browser::Move::History)
            .unwrap();
        let forest = ForestProjection::from_snapshot(
            browser.snapshot().unwrap(),
            browser.tree_states().unwrap(),
        )
        .unwrap();
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            loaded: HashSet::new(),
            live: HashSet::new(),
            forest,
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: Vec::new(),
        };
        let value = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        let tabs = value["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["active"], false);
        assert_ne!(tabs[0]["id"], current.root().to_string());
    }

    #[test]
    fn settled_current_pages_stay_scoped_during_a_navigation() {
        let (forest, ids) = chain_with_lifecycle(crate::browser::Spot::History { settled_at: 1 });
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            loaded: HashSet::new(),
            live: HashSet::new(),
            forest,
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: Vec::new(),
        };
        let value = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(value["tabs"], json!([]));
        assert_eq!(value["current"], ids[1].to_string());
        assert_eq!(value["visits"].as_object().unwrap().len(), 3);
    }
    #[test]
    fn the_frame_carries_every_visit_and_the_folded_strip() {
        let (forest, ids) = chain();
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            loaded: HashSet::new(),
            live: HashSet::new(),
            forest,
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: Vec::new(),
        };
        let value = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(value["current"], ids[1].to_string());
        assert_eq!(value["visits"].as_object().unwrap().len(), 3);
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], true);
        assert_eq!(
            value["visits"][ids[0].to_string()]["site"],
            format!("example.org · {}", ago(0))
        );
        assert_eq!(value["tabs"][0]["pending"], 1);
        assert_eq!(value["strip"]["height"], graph::REST_HEIGHT);
        let pill = value["strip"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "pill")
            .unwrap();
        assert_eq!(pill["id"], ids[1].to_string());
        assert_eq!(value["theme"], "dark");
        assert!(value["mail"]["pin"].is_null());
        assert!(value.get("forest").is_none());
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
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            loaded: HashSet::new(),
            live: HashSet::new(),
            forest,
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
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
        let value = super::frame(
            &frame,
            &[],
            (1240., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
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
        let strip = strip(&forest, &crate::mail::Mail::default(), 1240.);
        let edge = &strip["edges"][0];
        assert_eq!(edge["c"].as_array().unwrap().len(), 4);
        assert_eq!(edge["c"][0].as_array().unwrap().len(), 2);
    }
    #[test]
    fn inactive_pages_agree_in_counts_menus_strip_palette_and_zoomed_tree() {
        let (original, ids) = chain();
        let mut visits: Vec<_> = ids
            .iter()
            .map(|id| original.visit(*id).unwrap().clone())
            .collect();
        visits[1].branch_settled_at = Some(100);
        let mut dismissed = visits[2].clone();
        dismissed.id = ulid::Ulid::new().to_string().parse().unwrap();
        dismissed.parent = Some(ids[0]);
        dismissed.dismissed_at = Some(100);
        let skipped = dismissed.id;
        visits.push(dismissed);
        let forest = ForestProjection::from_snapshot(
            Snapshot {
                visits,
                current: Some(ids[0]),
            },
            vec![TreeState {
                root: TreeId(ids[0]),
                last_visit: ids[0],
                name: None,

                last_event_at: None,
                spot: crate::browser::Spot::Tab {
                    pinned: false,
                    position: 0,
                },
            }],
        )
        .unwrap();
        assert_eq!(forest.tabs()[0].pending_count, 0);
        let frame = ChromeFrame {
            shown: Default::default(),
            mail: crate::mail::Mail::default(),
            forest,
            loaded: ids.iter().copied().collect(),
            live: HashSet::new(),
            capture_enabled: true,
            trees_help: true,
            theme: ThemeMode::Dark,
            declined: Default::default(),
            bookmarks: vec![],
            back: None,
            forward: None,
            pending: None,
            files: vec![],
        };
        let value = super::frame(
            &frame,
            &[],
            (800., 800.),
            &tree_view::Workbench::default(),
            Surfaces {
                start: true,
                palette: None,
            },
        );
        assert_eq!(value["waiting"]["ids"], json!([]));
        assert_eq!(value["visits"][ids[2].to_string()]["queued"], false);
        assert_eq!(value["visits"][ids[2].to_string()]["loaded"], false);
        assert_eq!(value["visits"][skipped.to_string()]["dismissed"], true);
        assert_eq!(
            value["visits"][ids[1].to_string()]["stub"],
            "2 pages put away"
        );
        assert_eq!(value["treeView"]["tiles"].as_array().unwrap().len(), 3);
        assert!(!value["treeView"]["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["id"] == ids[2].to_string()));
        let pill = value["strip"]["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|n| n["kind"] == "pill")
            .unwrap();
        assert_eq!(pill["branches"], json!({"pages":1,"waiting":0}));
        assert!(pill["rows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["detail"] == "2 pages put away"));
        assert!(
            super::super::palette::results("", vec![], vec![], &[], &frame.forest)
                .0
                .is_empty()
        );
    }
}

fn mail_value(mail: &crate::mail::Mail) -> Value {
    use crate::mail::Trouble;
    let unread = mail.unread();
    let accounts = plural(mail.accounts.len(), "Gmail account");
    let name = if unread == 0 {
        format!("No unread mail in {accounts}")
    } else {
        format!("{unread} unread in {accounts}")
    };
    let updated = format!("Last update {}", ago(mail.read_at));
    let notice = mail.trouble.map(|t| {
        format!(
            "{} {updated}.",
            match t {
                Trouble::Unreachable => "Could not reach Gmail.",
                Trouble::Unreadable => "Gmail's feed changed, and Canopy cannot read it.",
            }
        )
    });
    let pin = (!mail.accounts.is_empty()).then(|| {
        json!({
            "count": if unread == 0 { String::new() } else { unread.to_string() },
            "stale": mail.trouble.is_some(),
            "name": if mail.trouble.is_some() { format!("{name}. {updated}") } else { name },
        })
    });
    let mut value = json!({
        "pin": pin,
        "notice": notice,
        "accounts": mail.accounts.iter().map(|a| {
            let count = if a.unread == 0 { "No unread mail".to_owned() } else { format!("{} unread", a.unread) };
            let more = a.unread.saturating_sub(a.messages.len() as u32);
            json!({
                "account": a.address,
                "address": a.address,
                "count": count,
                "name": format!("Open the inbox of {}, {count}", a.address),
                "more": (more > 0).then(|| format!("{more} more in the inbox")),
                "rows": a.messages.iter().map(|m| {
                    let when = m.issued.map(ago).unwrap_or_default();
                    let note = m.unmarked.map(unmarked_words);
                    let name = format!("{}, {}, {when}", m.from, m.subject);
                    json!({"id": m.id.as_str(), "from": m.from, "subject": m.subject, "snippet": m.snippet,
                        "when": when, "name": note.map_or_else(|| name.clone(), |n| format!("{name}. {n}")),
                        "markName": (m.link != a.inbox).then(|| format!("Mark read: {}, {}", m.from, m.subject)),
                        "unmarked": note})
                }).collect::<Vec<_>>(),
            })
        }).collect::<Vec<_>>(),
    });
    if mail.accounts.len() >= 2 {
        let tab = |key: &str, label: &str, owner: &str, count: u32| {
            json!({
                "key": key,
                "label": label,
                "count": if count == 0 { String::new() } else { count.to_string() },
                "name": if count == 0 { format!("{owner}, no unread mail") } else { format!("{owner}, {count} unread") },
            })
        };
        let local = |address: &str| address.split('@').next().unwrap_or(address).to_owned();
        let mut tabs = vec![tab("all", "All", "All accounts", unread)];
        tabs.extend(mail.accounts.iter().map(|a| {
            let short = local(&a.address);
            let shared = mail
                .accounts
                .iter()
                .filter(|other| local(&other.address) == short)
                .count()
                > 1;
            tab(
                &a.address,
                if shared { &a.address } else { &short },
                &a.address,
                a.unread,
            )
        }));
        value["tabs"] = json!(tabs);
    }
    value
}

fn unmarked_words(why: crate::mail::Unmarked) -> &'static str {
    match why {
        crate::mail::Unmarked::StillUnread => "Not marked read: Gmail still shows it as unread.",
        crate::mail::Unmarked::SignIn => "Not marked read: Gmail asked to sign in again.",
        crate::mail::Unmarked::NoAnswer => "Not marked read: Gmail did not answer.",
        crate::mail::Unmarked::Unconfirmed => "Not marked read: Canopy could not check with Gmail.",
    }
}

#[cfg(test)]
mod mail_tests {
    use super::*;
    use crate::mail::{Mail, Trouble};

    #[test]
    fn marks_name_real_targets_and_explain_each_failure() {
        let mut mail = crate::mail::fixture();
        assert!(mail_value(&mail)["accounts"][0]["rows"][0]["markName"].is_null());
        mail.accounts[0].messages[0].link =
            url::Url::parse("https://mail.google.com/mail/u/0?message_id=111").unwrap();
        let row = |m: &crate::mail::Mail| mail_value(m)["accounts"][0]["rows"][0].clone();
        assert_eq!(row(&mail)["markName"], "Mark read: A sender, A subject");
        assert!(row(&mail)["unmarked"].is_null());
        for (why, words) in [
            (
                crate::mail::Unmarked::NoAnswer,
                "Not marked read: Gmail did not answer.",
            ),
            (
                crate::mail::Unmarked::SignIn,
                "Not marked read: Gmail asked to sign in again.",
            ),
            (
                crate::mail::Unmarked::StillUnread,
                "Not marked read: Gmail still shows it as unread.",
            ),
            (
                crate::mail::Unmarked::Unconfirmed,
                "Not marked read: Canopy could not check with Gmail.",
            ),
        ] {
            mail.accounts[0].messages[0].unmarked = Some(why);
            assert_eq!(row(&mail)["unmarked"], words);
            assert!(row(&mail)["name"].as_str().unwrap().ends_with(words));
        }
    }

    #[test]
    fn account_tabs_have_stable_keys_unambiguous_labels_and_unread_names() {
        let mut mail = crate::mail::fixture();
        assert!(mail_value(&Mail::default()).get("tabs").is_none());
        assert!(mail_value(&mail).get("tabs").is_none());
        let mut second = mail.accounts[0].clone();
        second.address = "bo@example.test".into();
        second.unread = 0;
        mail.accounts.push(second);
        assert_eq!(
            mail_value(&mail)["tabs"],
            json!([
                {"key":"all", "label":"All", "count":"3", "name":"All accounts, 3 unread"},
                {"key":"ana@example.test", "label":"ana", "count":"3", "name":"ana@example.test, 3 unread"},
                {"key":"bo@example.test", "label":"bo", "count":"", "name":"bo@example.test, no unread mail"}
            ])
        );
        mail.accounts[1].address = "ana@work.test".into();
        mail.accounts[0].unread = 0;
        let value = mail_value(&mail);
        assert_eq!(value["tabs"][0]["count"], "");
        assert_eq!(value["tabs"][0]["name"], "All accounts, no unread mail");
        assert_eq!(value["tabs"][1]["label"], "ana@example.test");
        assert_eq!(value["tabs"][2]["label"], "ana@work.test");
    }

    #[test]
    fn the_frame_hides_mail_without_accounts_and_names_counts_rows_and_staleness() {
        assert!(mail_value(&Mail::default())["pin"].is_null());
        let mut mail = crate::mail::fixture();
        let value = mail_value(&mail);
        assert_eq!(value["pin"]["count"], "3");
        assert_eq!(value["pin"]["name"], "3 unread in 1 Gmail account");
        assert_eq!(value["pin"]["stale"], false);
        assert_eq!(value["accounts"][0]["account"], "ana@example.test");
        assert_eq!(value["accounts"][0]["more"], "2 more in the inbox");
        assert_eq!(
            value["accounts"][0]["rows"][0]["when"],
            ago(mail.accounts[0].messages[0].issued.unwrap())
        );
        for (trouble, prefix) in [
            (Trouble::Unreachable, "Could not reach Gmail."),
            (Trouble::Unreadable, "Gmail's feed changed"),
        ] {
            mail.trouble = Some(trouble);
            let value = mail_value(&mail);
            assert_eq!(value["pin"]["stale"], true);
            assert!(value["notice"].as_str().unwrap().starts_with(prefix));
            assert!(value["pin"]["name"]
                .as_str()
                .unwrap()
                .contains("Last update"));
        }
        mail.accounts[0].unread = 0;
        let value = mail_value(&mail);
        assert_eq!(value["pin"]["count"], "");
        assert!(value["pin"]["name"]
            .as_str()
            .unwrap()
            .starts_with("No unread mail"));
        assert_eq!(value["accounts"][0]["count"], "No unread mail");
        assert!(value["accounts"][0]["more"].is_null());
    }
}
