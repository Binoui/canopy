mod live;
mod native;

use crate::{
    browser::{Browser, Landing, LoadTicket, NavigationTarget, ThemeMode, TreeId, VisitId},
    chrome::{Favicons, CHROME_HEIGHT},
    forest::ForestProjection,
};
use anyhow::Result;
use live::LivePage;
pub use native::{CookieApplyError, DownloadEvent};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    rc::{Rc, Weak},
};
use url::Url;
use webkit2gtk::WebViewExt;

pub enum Go {
    Visit(VisitId),
    Parent,
    Tree(TreeId),
    Address { url: Url, target: NavigationTarget },
    Citation(VisitId),
    Saved(i64),
}

pub enum PageEvent {
    Changed,
    Loading(bool),
    Error(String),
    Remembered(i64),
    Download(DownloadEvent),
    OpenBranchPalette { visit: VisitId },
}

enum Continuation {
    BranchPalette,
    Path(Url),
}
struct PendingContinuation {
    state: ContinuationState,
    action: Continuation,
}

enum ContinuationState {
    Waiting(LoadTicket),
    Committed { ticket: LoadTicket, visit: VisitId },
}

impl ContinuationState {
    fn invalidated_by(&self, pending: Option<LoadTicket>) -> bool {
        match *self {
            Self::Waiting(ticket) => pending != Some(ticket),
            Self::Committed { ticket, .. } => pending.is_some_and(|next| next != ticket),
        }
    }
}

#[derive(Clone, Copy)]
enum StartPage {
    Reload,
    Show(VisitId),
    Blank,
}

type Observer = Rc<dyn Fn(PageEvent)>;

pub struct Pages {
    me: Weak<Pages>,
    browser: RefCell<Browser>,
    context: Rc<native::PageContext>,
    live: RefCell<Option<Rc<LivePage>>>,
    continuation: RefCell<Option<PendingContinuation>>,
    observer: RefCell<Option<Observer>>,
    start: Cell<Option<StartPage>>,
}

impl Pages {
    pub fn mount(container: &gtk::Fixed, browser: Browser, profile: &Path) -> Result<Rc<Self>> {
        let context = native::PageContext::new(profile)?;
        let pages = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            browser: RefCell::new(browser),
            context,
            live: RefCell::new(None),
            continuation: RefCell::new(None),
            observer: RefCell::new(None),
            start: Cell::new(None),
        });
        let page = LivePage::new(container, Rc::downgrade(&pages), &pages.context)?;
        *pages.live.borrow_mut() = Some(Rc::new(page));
        let weak = Rc::downgrade(&pages);
        pages.context.observe(move |event| {
            if let Some(pages) = weak.upgrade() {
                pages.receive(event);
            }
        });
        let weak = Rc::downgrade(&pages);
        pages
            .context
            .record_downloads(move |visit, path, size, source| {
                let pages = weak
                    .upgrade()
                    .ok_or_else(|| anyhow::anyhow!("The window was closed."))?;
                let result = pages
                    .browser
                    .borrow_mut()
                    .record_download(visit, path, size, source);
                result.map(|_| ())
            });
        Ok(pages)
    }

    fn page(&self) -> Rc<LivePage> {
        self.live.borrow().as_ref().expect("mounted page").clone()
    }

    pub fn read<T>(&self, query: impl FnOnce(&Browser) -> T) -> T {
        query(&self.browser.borrow())
    }

    pub fn favicons(&self) -> Rc<Favicons> {
        self.context.favicons.clone()
    }

    pub fn sandbox_enabled(&self) -> bool {
        use webkit2gtk::WebContextExt;
        self.context.native.is_sandbox_enabled()
    }

    pub fn observe(&self, callback: impl Fn(PageEvent) + 'static) {
        *self.observer.borrow_mut() = Some(Rc::new(callback));
    }

    // Clone the observer and end every state borrow before WebKit, GTK or Ui can reenter.
    fn receive(&self, event: PageEvent) {
        if matches!(event, PageEvent::Error(_)) {
            self.cancel_continuation();
        }
        if matches!(event, PageEvent::Loading(false)) {
            let ticket = self
                .continuation
                .borrow()
                .as_ref()
                .and_then(|c| match c.state {
                    ContinuationState::Committed { ticket, .. } => Some(ticket),
                    ContinuationState::Waiting(_) => None,
                });
            if let Some(ticket) = ticket {
                self.schedule_continuation(ticket);
            }
        }
        let pending = self
            .live
            .borrow()
            .as_ref()
            .and_then(|page| page.pending_ticket());
        if self
            .continuation
            .borrow()
            .as_ref()
            .is_some_and(|c| c.state.invalidated_by(pending))
        {
            self.continuation.borrow_mut().take();
        }
        let observer = self.observer.borrow().clone();
        if let Some(observer) = observer {
            observer(event);
        }
    }

    fn committed(self: &Rc<Self>, ticket: LoadTicket, visit: VisitId) {
        {
            let mut continuation = self.continuation.borrow_mut();
            let Some(c) = continuation.as_mut() else {
                return;
            };
            if !matches!(c.state, ContinuationState::Waiting(waiting) if waiting == ticket) {
                continuation.take();
                return;
            }
            c.state = ContinuationState::Committed { ticket, visit };
        }
        self.schedule_continuation(ticket);
    }

    fn schedule_continuation(&self, ticket: LoadTicket) {
        // Keep the existing loading gate, and let native commit handling finish binding first.
        let weak = self.me.clone();
        glib::idle_add_local_once(move || {
            let Some(pages) = weak.upgrade() else { return };
            if pages.page().native.is_loading() {
                return;
            }
            let continuation = {
                let mut pending = pages.continuation.borrow_mut();
                if pending.as_ref().is_some_and(|c| matches!(c.state, ContinuationState::Committed { ticket: committed, .. } if committed == ticket)) {
                    pending.take()
                } else { None }
            };
            let Some(PendingContinuation {
                state: ContinuationState::Committed { visit, .. },
                action,
            }) = continuation
            else {
                return;
            };
            match action {
                Continuation::BranchPalette => {
                    pages.receive(PageEvent::OpenBranchPalette { visit })
                }
                Continuation::Path(second) => {
                    let result = pages.go(Go::Address {
                        url: second,
                        target: NavigationTarget::ChildOf(visit),
                    });
                    if let Err(error) = result {
                        pages.receive(PageEvent::Error(error.to_string()));
                    }
                }
            }
        });
    }

    pub fn cancel_continuation(&self) {
        self.continuation.borrow_mut().take();
    }

    pub fn go(&self, go: Go) -> Result<()> {
        self.cancel_continuation();
        match go {
            Go::Visit(visit) => self.page().restore(visit),
            Go::Parent => self.page().back(),
            Go::Tree(tree) => {
                let visit = self.read(|b| b.tree_target(tree))?;
                self.page().restore(visit)
            }
            Go::Address { url, target } => self.page().navigate(url, target),
            Go::Citation(visit) => self.page().open_citation(visit),
            Go::Saved(id) => {
                let capture = self.read(|b| b.capture(id))?;
                self.page().open_saved(&capture)
            }
        }
    }

    pub fn branch_from_queued(&self, visit: VisitId) -> Result<()> {
        self.go(Go::Visit(visit))?;
        self.continue_with(Continuation::BranchPalette);
        Ok(())
    }
    pub fn open_path(&self, first: Url, second: Url) -> Result<()> {
        self.go(Go::Address {
            url: first,
            target: NavigationTarget::Root,
        })?;
        self.continue_with(Continuation::Path(second));
        Ok(())
    }
    fn continue_with(&self, action: Continuation) {
        *self.continuation.borrow_mut() =
            self.page()
                .pending_ticket()
                .map(|ticket| PendingContinuation {
                    state: ContinuationState::Waiting(ticket),
                    action,
                });
    }

    pub fn settle_tree(&self, tree: TreeId) -> Result<()> {
        self.cancel_continuation();
        let landing = self.browser.borrow_mut().settle_tree(tree)?;
        self.receive(PageEvent::Changed);
        match landing {
            Landing::Unchanged => Ok(()),
            Landing::Restore(visit) => self.go(Go::Visit(visit)),
            Landing::Start => {
                self.page().blank();
                Ok(())
            }
        }
    }

    pub fn prepare_resume(&self, urls: &[Url]) -> Result<()> {
        if !urls.is_empty() {
            self.start.set(Some(StartPage::Reload));
            return Ok(());
        }
        let forest =
            self.read(|b| ForestProjection::from_snapshot(b.snapshot()?, b.tree_states()?))?;
        let mut start = StartPage::Reload;
        if let Some(tree) = forest.current().and_then(|id| forest.tree_of(id)) {
            if forest
                .trees()
                .iter()
                .any(|t| t.id == tree && !t.lifecycle.is_open())
            {
                start = match self.browser.borrow_mut().settle_tree(tree)? {
                    Landing::Restore(visit) => StartPage::Show(visit),
                    Landing::Start => StartPage::Blank,
                    Landing::Unchanged => StartPage::Reload,
                };
                self.receive(PageEvent::Changed);
            }
        }
        self.start.set(Some(start));
        Ok(())
    }
    pub fn resume(&self) -> Result<()> {
        match self.start.take() {
            Some(StartPage::Reload) => self.reload(),
            Some(StartPage::Show(visit)) => self.go(Go::Visit(visit)),
            Some(StartPage::Blank) => {
                self.cancel_continuation();
                self.page().blank();
                Ok(())
            }
            None => Ok(()),
        }
    }
    pub fn open_roots(&self, urls: &[Url]) -> Result<()> {
        let roots = self.browser.borrow_mut().add_roots(urls)?;
        let Some(last) = roots.last().copied() else {
            return Ok(());
        };
        self.cancel_continuation();
        self.receive(PageEvent::Changed);
        if self.start.get().is_some() {
            self.start.set(Some(StartPage::Show(last)));
            Ok(())
        } else {
            self.go(Go::Visit(last))
        }
    }
    pub fn reload(&self) -> Result<()> {
        self.cancel_continuation();
        self.page().reload()
    }
    pub fn remember_selection(&self) -> Result<()> {
        self.page().remember_selection()
    }
    pub fn set_capture_enabled(&self, enabled: bool) -> Result<()> {
        self.page().set_capture_enabled(enabled)
    }
    pub fn set_theme(&self, mode: ThemeMode) -> Result<()> {
        self.browser.borrow_mut().set_theme_mode(mode)
    }
    pub fn rename_tree(&self, tree: TreeId, name: &str) -> Result<()> {
        self.browser.borrow_mut().rename_tree(tree, name)
    }
    pub fn focus(&self) {
        self.page().focus();
    }
    pub fn resize(&self, width: f64, height: f64) {
        self.page()
            .set_bounds(0., CHROME_HEIGHT, width, height - CHROME_HEIGHT);
    }
    pub fn apply_cookie_batches(
        &self,
        batches: Vec<crate::cookies::Batch>,
        done: impl FnOnce(std::result::Result<usize, CookieApplyError>) + 'static,
    ) {
        self.context.apply_cookie_batches(batches, done);
    }
    #[cfg(feature = "gui-smoke")]
    pub fn release_selected_page(&self) {
        let page = self.live.borrow_mut().take();
        drop(page);
    }
    #[cfg(feature = "gui-smoke")]
    pub fn mount_selected_page(self: &Rc<Self>, container: &gtk::Fixed) -> Result<()> {
        let page = LivePage::new(container, Rc::downgrade(self), &self.context)?;
        *self.live.borrow_mut() = Some(Rc::new(page));
        Ok(())
    }
    #[cfg(feature = "gui-smoke")]
    pub fn selected_page(&self) -> SelectedPage<'_> {
        SelectedPage {
            pages: self,
            native: self.page().native.clone(),
        }
    }
}

#[cfg(feature = "gui-smoke")]
pub struct SelectedPage<'a> {
    pages: &'a Pages,
    pub native: webkit2gtk::WebView,
}
#[cfg(feature = "gui-smoke")]
impl SelectedPage<'_> {
    pub fn navigate(&self, url: Url, target: NavigationTarget) -> Result<()> {
        self.pages.go(Go::Address { url, target })
    }
    pub fn restore(&self, visit: VisitId) -> Result<()> {
        self.pages.go(Go::Visit(visit))
    }
    pub fn back(&self) -> Result<()> {
        self.pages.go(Go::Parent)
    }
    pub fn focus(&self) {
        self.pages.focus();
    }
    pub fn open_citation(&self, visit: VisitId) -> Result<()> {
        self.pages.go(Go::Citation(visit))
    }
    pub fn reload(&self) -> Result<()> {
        self.pages.reload()
    }
    pub fn capture_enabled(&self) -> Result<bool> {
        self.pages.read(|b| b.capture_enabled())
    }
    pub fn remember_selection(&self) -> Result<()> {
        self.pages.remember_selection()
    }
}

#[cfg(test)]
mod continuation_tests {
    use super::*;
    use crate::browser::Navigation;

    #[test]
    fn waiting_continuation_requires_its_own_attempt() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let first = navigation.native_request(url.clone());
        let state = ContinuationState::Waiting(first);
        assert!(!state.invalidated_by(Some(first)));
        let replacement = navigation.native_request(url);
        assert!(state.invalidated_by(Some(replacement)));
        navigation.failed(replacement);
        assert!(state.invalidated_by(navigation.pending_ticket()));
    }

    #[test]
    fn cancellation_and_blank_invalidate_waiting_continuation() {
        let mut navigation = Navigation::default();
        let ticket = navigation.native_request(Url::parse("https://example.org/").unwrap());
        let state = ContinuationState::Waiting(ticket);
        navigation.clear();
        assert!(state.invalidated_by(navigation.pending_ticket()));
    }

    #[test]
    fn committed_continuation_survives_completion_but_not_replacement() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.org/").unwrap();
        let ticket = navigation.native_request(url.clone());
        let directory = tempfile::tempdir().unwrap();
        let mut browser = Browser::open(&directory.path().join("forest.sqlite3")).unwrap();
        let visit = browser.add_roots(std::slice::from_ref(&url)).unwrap()[0];
        let state = ContinuationState::Committed { ticket, visit };
        assert!(!state.invalidated_by(None));
        assert!(state.invalidated_by(Some(navigation.native_request(url))));
    }
}
