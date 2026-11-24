//! Find in page. Each view holds at most one find session, for the document it showed
//! when find opened.
//!
//! WebKit answers each search or step with one signal that carries only the match count:
//! no request identity and no match index. So a view keeps one request in flight, and
//! Canopy counts the position itself. A new query starts from a cleared selection, at
//! match 1. A step starts from the match WebKit last selected, which Canopy keeps in its
//! isolated world, so a click in the page between steps cannot shift the count. A changed
//! total starts the query over. `tests/find_smoke.rs` checks these WebKit facts.

use std::{
    collections::VecDeque,
    num::NonZeroU32,
    rc::{Rc, Weak},
    sync::atomic::{AtomicU64, Ordering},
};

use webkit2gtk::{FindControllerExt, WebViewExt};

use super::{LivePage, NativeState, WORLD};
use crate::{
    browser::VisitId,
    pages::{PageEvent, Pages},
};

/// One find session. The chrome sends its number back with every edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FindId(u64);

impl FindId {
    fn next() -> Self {
        static SESSIONS: AtomicU64 = AtomicU64::new(1);
        Self(SESSIONS.fetch_add(1, Ordering::Relaxed))
    }

    pub fn get(self) -> u64 {
        self.0
    }
}

impl From<u64> for FindId {
    fn from(number: u64) -> Self {
        Self(number)
    }
}

/// The session a command is for: the selected page's, or the one the chrome names. A
/// named session that has closed takes no command.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindTarget {
    Selected,
    Session(FindId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FindAction {
    Open,
    Query(String),
    Next,
    Previous,
    Close,
}

/// The selected match among all matches.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MatchPosition {
    /// From 1 to `total`.
    index: u32,
    total: NonZeroU32,
}

impl MatchPosition {
    fn first(total: NonZeroU32) -> Self {
        Self { index: 1, total }
    }

    pub fn index(self) -> u32 {
        self.index
    }

    pub fn total(self) -> u32 {
        self.total.get()
    }

    fn step(self, direction: Direction) -> Self {
        let total = self.total.get();
        let index = match direction {
            Direction::Next if self.index == total => 1,
            Direction::Next => self.index + 1,
            Direction::Previous if self.index == 1 => total,
            Direction::Previous => self.index - 1,
        };
        Self { index, ..self }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindMatches {
    EmptyQuery,
    Pending,
    NoMatches,
    Position(MatchPosition),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FindReadout {
    pub id: FindId,
    /// The query `matches` describes.
    pub query: String,
    pub matches: FindMatches,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FindClose {
    User,
    /// The page switched or loaded another document.
    ContextChanged,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FindUpdate {
    /// Find opened, or opened again: the bar takes the keyboard.
    Focused(FindReadout),
    Shown(FindReadout),
    Closed(FindClose),
}

/// The document a session belongs to. A same-document address change keeps both
/// fields; a new visit or a full load changes one of them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct Owner {
    visit: VisitId,
    /// `NativeState::history_document`.
    document: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Direction {
    Next,
    Previous,
}

/// WebKit's answer to a search or a step.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reply {
    Found(NonZeroU32),
    Missing,
}

/// What the view has to do after a driver transition, in order.
#[derive(Debug, Eq, PartialEq)]
pub(super) enum Effect {
    Search(String),
    Step(Direction),
    /// Keep the match WebKit just selected.
    Remember,
    /// Clear WebKit's highlights.
    Finish,
    Publish(FindUpdate),
}

#[derive(Default)]
pub(super) struct Driver {
    session: Option<Session>,
    flight: Option<Flight>,
}

struct Session {
    id: FindId,
    owner: Owner,
    query: String,
    /// Counts query edits, so a reply for an older query is recognized.
    revision: u64,
    matches: FindMatches,
    unsearched: bool,
    /// Steps asked for while the current query's search was still running.
    steps: VecDeque<Direction>,
}

/// The request WebKit has not answered yet. It outlives a close or a newer query: its
/// answer would otherwise be taken for the next request's.
#[derive(Clone, Copy)]
struct Flight {
    session: FindId,
    revision: u64,
    operation: Operation,
}

#[derive(Clone, Copy)]
enum Operation {
    Search,
    Step(Direction),
}

impl Session {
    fn readout(&self) -> FindReadout {
        FindReadout {
            id: self.id,
            query: self.query.clone(),
            matches: self.matches,
        }
    }
}

impl Driver {
    /// Closes a session whose document is no longer `owner`, the one on screen.
    pub(super) fn reconcile(&mut self, owner: Option<Owner>) -> Vec<Effect> {
        if self
            .session
            .as_ref()
            .is_some_and(|s| Some(s.owner) != owner)
        {
            return self.close(FindTarget::Selected, FindClose::ContextChanged);
        }
        Vec::new()
    }

    pub(super) fn open(&mut self, owner: Option<Owner>) -> Vec<Effect> {
        let Some(owner) = owner else {
            return Vec::new();
        };
        let session = self.session.get_or_insert_with(|| Session {
            id: FindId::next(),
            owner,
            query: String::new(),
            revision: 0,
            matches: FindMatches::EmptyQuery,
            unsearched: false,
            steps: VecDeque::new(),
        });
        vec![Effect::Publish(FindUpdate::Focused(session.readout()))]
    }

    pub(super) fn query(&mut self, target: FindTarget, text: String) -> Vec<Effect> {
        let Some(session) = self.session(target).filter(|s| s.query != text) else {
            return Vec::new();
        };
        session.query = text;
        session.revision += 1;
        session.steps.clear();
        session.unsearched = !session.query.is_empty();
        let mut effects = Vec::new();
        session.matches = if session.unsearched {
            FindMatches::Pending
        } else {
            effects.push(Effect::Finish);
            FindMatches::EmptyQuery
        };
        effects.push(Effect::Publish(FindUpdate::Shown(session.readout())));
        self.dispatch(&mut effects);
        effects
    }

    pub(super) fn step(&mut self, target: FindTarget, direction: Direction) -> Vec<Effect> {
        let Some(session) = self
            .session(target)
            .filter(|s| matches!(s.matches, FindMatches::Pending | FindMatches::Position(_)))
        else {
            return Vec::new();
        };
        session.steps.push_back(direction);
        let mut effects = Vec::new();
        self.dispatch(&mut effects);
        effects
    }

    pub(super) fn close(&mut self, target: FindTarget, reason: FindClose) -> Vec<Effect> {
        if self.session(target).is_none() {
            return Vec::new();
        }
        self.session = None;
        vec![Effect::Finish, Effect::Publish(FindUpdate::Closed(reason))]
    }

    pub(super) fn completed(&mut self, reply: Reply) -> Vec<Effect> {
        let Some(flight) = self.flight.take() else {
            return Vec::new();
        };
        let mut effects = Vec::new();
        if let Some(session) = self
            .session
            .as_mut()
            .filter(|s| s.id == flight.session && s.revision == flight.revision)
        {
            session.matches = match (reply, flight.operation, session.matches) {
                (Reply::Missing, ..) => {
                    session.steps.clear();
                    FindMatches::NoMatches
                }
                (Reply::Found(total), Operation::Search, _) => {
                    effects.push(Effect::Remember);
                    FindMatches::Position(MatchPosition::first(total))
                }
                (Reply::Found(total), Operation::Step(direction), FindMatches::Position(at))
                    if at.total == total =>
                {
                    effects.push(Effect::Remember);
                    FindMatches::Position(at.step(direction))
                }
                (Reply::Found(_), Operation::Step(_), _) => {
                    session.steps.clear();
                    session.unsearched = true;
                    FindMatches::Pending
                }
            };
            effects.push(Effect::Publish(FindUpdate::Shown(session.readout())));
        }
        self.dispatch(&mut effects);
        effects
    }

    fn session(&mut self, target: FindTarget) -> Option<&mut Session> {
        self.session.as_mut().filter(|s| match target {
            FindTarget::Selected => true,
            FindTarget::Session(id) => s.id == id,
        })
    }

    /// Sends the next request once WebKit has answered the last one.
    fn dispatch(&mut self, effects: &mut Vec<Effect>) {
        if self.flight.is_some() {
            return;
        }
        let Some(session) = self.session.as_mut() else {
            return;
        };
        let operation = match (session.unsearched, session.matches) {
            (true, _) => {
                session.unsearched = false;
                effects.push(Effect::Search(session.query.clone()));
                Operation::Search
            }
            (false, FindMatches::Position(_)) => match session.steps.pop_front() {
                Some(direction) => {
                    effects.push(Effect::Step(direction));
                    Operation::Step(direction)
                }
                None => return,
            },
            _ => return,
        };
        self.flight = Some(Flight {
            session: session.id,
            revision: session.revision,
            operation,
        });
    }
}

/// Clears the page's selection, so WebKit's search starts at the top of the document.
const FROM_THE_TOP: &str = "getSelection().removeAllRanges()";
const REMEMBER: &str = "{const s=getSelection();globalThis.__canopyFindMatch=s.rangeCount?s.getRangeAt(0).cloneRange():null}";
const RESTORE: &str = "{const r=globalThis.__canopyFindMatch;if(r?.startContainer.isConnected){const s=getSelection();s.removeAllRanges();s.addRange(r)}}";

impl Pages {
    /// Opens, edits, steps or closes find on the selected page.
    pub fn find(&self, target: FindTarget, action: FindAction) {
        let Some(page) = self.branches.live.borrow().clone() else {
            return;
        };
        let state = &page.state;
        let owner = state.find_owner();
        let effects = {
            let mut driver = state.find.borrow_mut();
            let mut effects = driver.reconcile(owner);
            effects.extend(match action {
                FindAction::Open => driver.open(owner),
                FindAction::Query(text) => driver.query(target, text),
                FindAction::Next => driver.step(target, Direction::Next),
                FindAction::Previous => driver.step(target, Direction::Previous),
                FindAction::Close => driver.close(target, FindClose::User),
            });
            effects
        };
        state.run_find(effects);
    }

    fn publish_find(&self, update: FindUpdate) {
        let observer = self.observer.borrow().clone();
        if let Some(observer) = observer {
            observer(PageEvent::Find(update));
        }
    }
}

impl LivePage {
    /// Ends find on a page that is leaving the screen.
    pub(in crate::pages) fn close_find(&self) {
        let effects = self
            .state
            .find
            .borrow_mut()
            .close(FindTarget::Selected, FindClose::ContextChanged);
        self.state.run_find(effects);
    }
}

impl NativeState {
    fn find_owner(&self) -> Option<Owner> {
        let visit = self.navigation.borrow().current.as_ref()?.visit;
        Some(Owner {
            visit,
            document: self.history_document.get(),
        })
    }

    /// Ends find once the document it opened on has been replaced.
    pub(super) fn reconcile_find(&self) {
        let effects = self.find.borrow_mut().reconcile(self.find_owner());
        self.run_find(effects);
    }

    fn run_find(&self, effects: Vec<Effect>) {
        let script = |source: &str| {
            self.native.evaluate(source, WORLD, None, None);
        };
        // WebKit handles scripts and find requests in the order they are sent.
        for effect in effects {
            match effect {
                Effect::Search(query) => {
                    script(FROM_THE_TOP);
                    self.native.find(&query);
                }
                Effect::Step(direction) => {
                    script(RESTORE);
                    match direction {
                        Direction::Next => self.native.find_next(),
                        Direction::Previous => self.native.find_previous(),
                    }
                }
                Effect::Remember => script(REMEMBER),
                Effect::Finish => self.native.find_finish(),
                Effect::Publish(update) => self.owner().publish_find(update),
            }
        }
    }
}

/// Connects WebKit's answers to the view's driver.
pub(super) fn setup(state: &Rc<NativeState>) -> Vec<(glib::Object, glib::SignalHandlerId)> {
    use glib::Cast;
    let Some(controller) = super::temporary_webkit_view(&*state.native).find_controller() else {
        return Vec::new();
    };
    let answer = |weak: &Weak<NativeState>, reply: Reply| {
        if let Some(state) = Pages::enter_native(weak) {
            let effects = state.find.borrow_mut().completed(reply);
            state.run_find(effects);
        }
    };
    let weak = Rc::downgrade(state);
    let found = controller.connect_found_text(move |_, count| {
        answer(
            &weak,
            NonZeroU32::new(count).map_or(Reply::Missing, Reply::Found),
        );
    });
    let weak = Rc::downgrade(state);
    let failed = controller.connect_failed_to_find_text(move |_| answer(&weak, Reply::Missing));
    let object = controller.upcast::<glib::Object>();
    vec![(object.clone(), found), (object, failed)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn owner(document: u64) -> Option<Owner> {
        Some(Owner {
            visit: "01JABCDEFGHJKMNPQRSTVWXYZ0".parse().unwrap(),
            document,
        })
    }

    fn found(total: u32) -> Reply {
        Reply::Found(NonZeroU32::new(total).unwrap())
    }

    fn shown(effects: &[Effect]) -> Option<FindMatches> {
        effects.iter().rev().find_map(|effect| match effect {
            Effect::Publish(FindUpdate::Shown(readout)) => Some(readout.matches),
            _ => None,
        })
    }

    fn at(index: u32, total: u32) -> FindMatches {
        FindMatches::Position(MatchPosition {
            index,
            total: NonZeroU32::new(total).unwrap(),
        })
    }

    fn opened() -> (Driver, FindId) {
        let mut driver = Driver::default();
        let effects = driver.open(owner(1));
        let [Effect::Publish(FindUpdate::Focused(readout))] = effects.as_slice() else {
            panic!("open focuses the bar: {effects:?}");
        };
        assert_eq!(readout.matches, FindMatches::EmptyQuery);
        let id = readout.id;
        (driver, id)
    }

    fn searched(total: u32) -> (Driver, FindId) {
        let (mut driver, id) = opened();
        driver.query(FindTarget::Session(id), "canopy".into());
        driver.completed(found(total));
        (driver, id)
    }

    #[test]
    fn a_new_query_starts_at_the_first_match() {
        let (mut driver, id) = opened();
        let effects = driver.query(FindTarget::Session(id), "canopy".into());
        assert_eq!(shown(&effects), Some(FindMatches::Pending));
        assert!(effects.contains(&Effect::Search("canopy".into())));
        let effects = driver.completed(found(12));
        assert_eq!(shown(&effects), Some(at(1, 12)));
        assert!(effects.contains(&Effect::Remember));
    }

    #[test]
    fn steps_wrap_both_ways() {
        let (mut driver, id) = searched(3);
        let mut index = Vec::new();
        for direction in [
            Direction::Next,
            Direction::Next,
            Direction::Next,
            Direction::Previous,
            Direction::Previous,
        ] {
            let effects = driver.step(FindTarget::Session(id), direction);
            assert_eq!(effects, [Effect::Step(direction)]);
            match shown(&driver.completed(found(3))) {
                Some(FindMatches::Position(at)) => index.push(at.index()),
                other => panic!("a step lands on a match: {other:?}"),
            }
        }
        assert_eq!(index, [2, 3, 1, 3, 2]);
    }

    #[test]
    fn one_request_is_in_flight_and_typing_coalesces() {
        let (mut driver, id) = opened();
        let target = FindTarget::Session(id);
        assert!(driver
            .query(target, "c".into())
            .contains(&Effect::Search("c".into())));
        for text in ["ca", "can", "cano"] {
            let effects = driver.query(target, text.into());
            assert!(!effects.iter().any(|e| matches!(e, Effect::Search(_))));
            assert_eq!(shown(&effects), Some(FindMatches::Pending));
        }
        let effects = driver.completed(found(40));
        assert_eq!(
            shown(&effects),
            None,
            "the reply for c is not shown for cano"
        );
        assert_eq!(effects, [Effect::Search("cano".into())]);
        assert_eq!(shown(&driver.completed(found(5))), Some(at(1, 5)));
    }

    #[test]
    fn steps_asked_during_a_search_run_after_it() {
        let (mut driver, id) = opened();
        let target = FindTarget::Session(id);
        driver.query(target, "canopy".into());
        assert!(driver.step(target, Direction::Next).is_empty());
        assert!(driver.step(target, Direction::Next).is_empty());
        let effects = driver.completed(found(12));
        assert_eq!(shown(&effects), Some(at(1, 12)));
        assert_eq!(effects.last(), Some(&Effect::Step(Direction::Next)));
        let effects = driver.completed(found(12));
        assert_eq!(effects.last(), Some(&Effect::Step(Direction::Next)));
        assert_eq!(shown(&driver.completed(found(12))), Some(at(3, 12)));
    }

    #[test]
    fn a_changed_total_starts_the_query_over() {
        let (mut driver, id) = searched(12);
        let target = FindTarget::Session(id);
        driver.step(target, Direction::Next);
        driver.step(target, Direction::Next);
        let effects = driver.completed(found(13));
        assert_eq!(shown(&effects), Some(FindMatches::Pending));
        assert_eq!(effects.last(), Some(&Effect::Search("canopy".into())));
        assert_eq!(shown(&driver.completed(found(13))), Some(at(1, 13)));
        assert!(
            driver.completed(found(13)).is_empty(),
            "the queued step was dropped"
        );
    }

    #[test]
    fn no_matches_and_empty_queries_take_no_steps() {
        let (mut driver, id) = opened();
        let target = FindTarget::Session(id);
        assert!(driver.step(target, Direction::Next).is_empty());
        driver.query(target, "zebra".into());
        assert_eq!(
            shown(&driver.completed(Reply::Missing)),
            Some(FindMatches::NoMatches)
        );
        assert!(driver.step(target, Direction::Next).is_empty());
        let effects = driver.query(target, String::new());
        assert_eq!(effects[0], Effect::Finish);
        assert_eq!(shown(&effects), Some(FindMatches::EmptyQuery));
    }

    #[test]
    fn a_closed_session_ignores_its_late_reply_and_named_commands() {
        let (mut driver, id) = opened();
        let target = FindTarget::Session(id);
        driver.query(target, "canopy".into());
        assert_eq!(
            driver.close(target, FindClose::User),
            [
                Effect::Finish,
                Effect::Publish(FindUpdate::Closed(FindClose::User))
            ]
        );
        assert!(driver.query(target, "other".into()).is_empty());
        assert!(driver.close(target, FindClose::User).is_empty());
        let effects = driver.open(owner(1));
        let Some(Effect::Publish(FindUpdate::Focused(reopened))) = effects.first() else {
            panic!("open focuses the bar");
        };
        assert_ne!(reopened.id, id);
        let reopened = FindTarget::Session(reopened.id);
        assert!(
            driver.query(reopened, "forest".into()).len() == 1,
            "the new search waits for the old request"
        );
        let effects = driver.completed(found(12));
        assert_eq!(effects, [Effect::Search("forest".into())]);
        assert_eq!(shown(&driver.completed(found(5))), Some(at(1, 5)));
    }

    #[test]
    fn another_document_closes_the_session() {
        let (mut driver, _) = searched(12);
        assert!(driver.reconcile(owner(1)).is_empty());
        assert_eq!(
            driver.reconcile(owner(2)),
            [
                Effect::Finish,
                Effect::Publish(FindUpdate::Closed(FindClose::ContextChanged))
            ]
        );
        assert!(driver.open(None).is_empty(), "no document, no find");
    }

    #[test]
    fn opening_again_keeps_the_session() {
        let (mut driver, id) = searched(4);
        let effects = driver.open(owner(1));
        assert_eq!(
            effects,
            [Effect::Publish(FindUpdate::Focused(FindReadout {
                id,
                query: "canopy".into(),
                matches: at(1, 4)
            }))]
        );
    }
}
