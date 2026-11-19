mod items;
mod place;

use super::{Reply, Room};
use crate::{
    browser::VisitId,
    chrome::{tree_card, view, ChromeFrame, ChromeIntent},
};
use anyhow::{Context, Result};
use items::Facts;
use place::{Layout, Point};
use serde_json::{json, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::chrome) enum Level {
    Far,
    Middle,
    Near,
}
struct Band {
    min: f64,
    max: f64,
    rest: f64,
}
impl Level {
    const ALL: [Self; 3] = [Self::Far, Self::Middle, Self::Near];
    fn name(self) -> &'static str {
        match self {
            Self::Far => "far",
            Self::Middle => "middle",
            Self::Near => "near",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Far => "Far",
            Self::Middle => "Middle",
            Self::Near => "Near",
        }
    }
    fn band(self) -> Band {
        match self {
            Self::Far => Band {
                min: 0.42,
                max: 0.8,
                rest: 0.62,
            },
            Self::Middle => Band {
                min: 0.8,
                max: 1.55,
                rest: 1.15,
            },
            Self::Near => Band {
                min: 1.55,
                max: 2.6,
                rest: 1.9,
            },
        }
    }
    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.name() == s)
    }
    fn hint(self) -> &'static str {
        match self {
            Self::Far => "every open tree, one mark per site",
            Self::Middle => "sites folded, the pages that matter named",
            Self::Near => "every page by name",
        }
    }
}
#[derive(Debug)]
pub(in crate::chrome) enum Input {
    Zoom {
        level: Level,
        request: u64,
        anchor: Option<VisitId>,
    },
    Go(VisitId),
}
impl Input {
    pub fn parse(m: &Value) -> Result<Option<Self>> {
        Ok(match m["do"].as_str() {
            Some("go") => Some(Self::Go(m["id"].as_str().context("Missing page")?.parse()?)),
            Some("zoom") => Some(Self::Zoom {
                level: Level::parse(m["level"].as_str().unwrap_or_default())
                    .context("Unknown zoom level")?,
                request: m["request"].as_u64().context("Missing zoom request")?,
                anchor: m["anchor"].as_str().map(str::parse).transpose()?,
            }),
            _ => None,
        })
    }
}
pub(super) struct State {
    level: Level,
    request: u64,
    anchor: Option<VisitId>,
    layout: Layout,
}
impl Default for State {
    fn default() -> Self {
        Self {
            level: Level::Middle,
            request: 0,
            anchor: None,
            layout: Layout::Flow,
        }
    }
}
impl State {
    pub fn input(&mut self, input: Input, frame: &ChromeFrame) -> Reply {
        match input {
            Input::Zoom {
                level,
                request,
                anchor,
            } => {
                if request > self.request {
                    self.level = level;
                    self.request = request;
                    self.anchor = anchor;
                }
            }
            Input::Go(id) => {
                if items::drawn(&frame.forest, id)
                    && frame
                        .forest
                        .tree_of(id)
                        .and_then(|t| frame.forest.tree(t))
                        .is_some_and(|t| t.place.is_tab())
                {
                    return if Some(id) == frame.forest.current() {
                        Reply::Return
                    } else {
                        Reply::Go(ChromeIntent::Restore(id))
                    };
                }
            }
        }
        Reply::Unchanged
    }
    pub fn value(&self, frame: &ChromeFrame, room: Room) -> Value {
        let forest = &frame.forest;
        let facts = Facts {
            forest,
            previous: frame.shown.previous(forest),
            live: &frame.live,
            shown: &frame.shown,
        };
        let current = forest.current().and_then(|id| forest.tree_of(id));
        let mut trees: Vec<_> = forest.tabs().iter().collect();
        trees.sort_by_key(|t| Some(t.id) != current);
        let folded: Vec<_> = trees
            .iter()
            .map(|t| items::fold(&facts, t, self.level))
            .collect();
        let placed = self.layout.place(self.level, &folded, room);
        let mut values = Vec::new();
        let mut points = Vec::new();
        let mut home = None;
        let mut anchor = None;
        let mut visits = serde_json::Map::new();
        for (t, tree) in trees.iter().enumerate() {
            let p = placed.trees[t];
            values.push(json!({"k":format!("t:{}",tree.id.root()),"kind":"tree","id":tree.id.root().to_string(),"t":tree.title,"sub":view::plural(tree.page_count,"page"),"current":Some(tree.id)==current,"menu":tree_card::menu(tree),"x":p.x,"y":p.y,"w":180,"h":24}));
            points.push(p);
            let offset = values.len();
            for (i, item) in folded[t].iter().enumerate() {
                let p = placed.items[t][i];
                let index = values.len();
                if item.appearance.current {
                    home = Some((t, i));
                }
                if self.anchor.is_some_and(|id| item.pages.contains(&id)) {
                    anchor = Some(index);
                }
                let mut value = item.paint(&facts, self.level);
                value["x"] = json!(p.x);
                value["y"] = json!(p.y);
                value["w"] = json!(item.width(self.level));
                value["h"] = json!(item.height(self.level));
                value["up"] = json!(item.parent.map(|p| p + offset));
                for id in &item.pages {
                    if Some(tree.id) != current {
                        visits.insert(
                            id.to_string(),
                            view::visit_value(
                                forest,
                                forest.visit(*id).unwrap(),
                                &frame.loaded,
                                &frame.live,
                            ),
                        );
                    }
                }
                points.push(Point { x: p.x, y: p.y });
                values.push(value);
            }
        }
        for (value, nav) in values.iter_mut().zip(place::neighbours(&points)) {
            value["nav"] = json!(nav);
        }
        let b = self.level.band();
        let back = facts
            .previous
            .and_then(|id| forest.visit(id))
            .map(|v| json!({"id":v.id.to_string(),"label":"Back to","title":v.label()}));
        json!({"level":self.level.name(),"layout":self.layout.name(),"hint":self.level.hint(),"items":values,"visits":visits,"back":back,"edge":self.layout.edge(),
            "css":self.level.metrics().css(),"levels":Level::ALL.map(|l|{let b=l.band();json!({"level":l.name(),"label":l.label(),"min":b.min,"max":b.max,"rest":b.rest})}),
            "legend":[["current","you are here"],["previous","where you were"],["open","still open"],["visited","visited"],["waiting","waiting"]],
            "help":"wheel zooms · drag moves · a click goes to the page",
            "place":{"request":self.request,"anchor":anchor,"page":self.anchor.map(|id|id.to_string()),"pan":place::camera(&placed,home,room,1.),"k":b.rest}})
    }
}

#[cfg(test)]
mod tests;
