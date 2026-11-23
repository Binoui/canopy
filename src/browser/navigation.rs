use super::VisitId;
use ulid::Ulid;
use url::Url;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ViewId(Ulid);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LoadTicket {
    view: ViewId,
    sequence: u64,
}

impl LoadTicket {
    pub fn belongs_to(self, navigation: &Navigation) -> bool {
        self.view == navigation.view
    }

    pub(crate) fn is_latest(self, navigation: &Navigation) -> bool {
        self.belongs_to(navigation) && self.sequence == navigation.sequence
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub view: ViewId,
    pub visit: VisitId,
    pub stored_url: Url,
    pub page_url: Url,
    pub epoch: u64,
}

#[cfg(feature = "desktop")]
impl Document {
    pub(crate) fn identity(&self) -> String {
        format!("{}:{}", self.view.0, self.epoch)
    }
}

/// Where the load in flight is headed, so the chrome can show it before it commits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PendingTarget {
    /// A page already in the forest.
    Visit(VisitId),
    /// A new tree, whose root does not exist until the page commits.
    NewTree(Url),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Selected,
    Hidden,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum NewVisitOrigin {
    Explicit,
    Page,
    FormPost,
}

#[derive(Clone, Debug)]
pub(super) enum Intent {
    New {
        parent: Option<VisitId>,
        source: Option<Box<super::SourceContext>>,
        origin: NewVisitOrigin,
    },
    Restore {
        visit: VisitId,
    },
}

#[derive(Clone, Debug, Default)]
pub(super) enum Pending {
    #[default]
    Idle,
    Requested {
        ticket: LoadTicket,
        intent: Intent,
        url: Url,
    },
    Loading {
        ticket: LoadTicket,
        intent: Intent,
        url: Url,
    },
}

pub struct Navigation {
    pub current: Option<Document>,
    pub(super) pending: Pending,
    view: ViewId,
    sequence: u64,
    epoch: u64,
}

impl Default for Navigation {
    fn default() -> Self {
        Self {
            current: None,
            pending: Pending::Idle,
            view: ViewId(Ulid::new()),
            sequence: 0,
            epoch: 0,
        }
    }
}

impl Navigation {
    pub(crate) fn view_id(&self) -> ViewId {
        self.view
    }

    pub(super) fn request(&mut self, intent: Intent, url: Url) -> LoadTicket {
        self.sequence += 1;
        let ticket = LoadTicket {
            view: self.view,
            sequence: self.sequence,
        };
        self.pending = Pending::Requested {
            ticket,
            intent,
            url,
        };
        ticket
    }

    pub(crate) fn pending_anchor(&self) -> Option<VisitId> {
        let intent = match &self.pending {
            Pending::Requested { intent, .. } | Pending::Loading { intent, .. } => intent,
            Pending::Idle => return None,
        };
        match intent {
            Intent::New { parent, .. } => *parent,
            Intent::Restore { visit } => Some(*visit),
        }
    }

    pub fn pending_target(&self) -> Option<PendingTarget> {
        match &self.pending {
            Pending::Requested { intent, url, .. } | Pending::Loading { intent, url, .. } => {
                match intent {
                    Intent::Restore { visit } => Some(PendingTarget::Visit(*visit)),
                    Intent::New { parent: None, .. } => Some(PendingTarget::NewTree(url.clone())),
                    Intent::New { .. } => None,
                }
            }
            Pending::Idle => None,
        }
    }

    pub(crate) fn pending_restore(&self) -> Option<VisitId> {
        match &self.pending {
            Pending::Requested {
                intent: Intent::Restore { visit },
                ..
            }
            | Pending::Loading {
                intent: Intent::Restore { visit },
                ..
            } => Some(*visit),
            _ => None,
        }
    }

    pub fn pending_ticket(&self) -> Option<LoadTicket> {
        match self.pending {
            Pending::Requested { ticket, .. } | Pending::Loading { ticket, .. } => Some(ticket),
            Pending::Idle => None,
        }
    }

    // Started can still report the previous URI; the issued address owns this attempt.
    pub fn started(&mut self, ticket: LoadTicket) {
        if self.pending_ticket() != Some(ticket) {
            return;
        }
        if let Pending::Requested { intent, url, .. } = &self.pending {
            self.pending = Pending::Loading {
                ticket,
                intent: intent.clone(),
                url: url.clone(),
            };
        }
    }

    pub(crate) fn request_queued(&mut self, visit: VisitId, url: Url) -> LoadTicket {
        self.request(Intent::Restore { visit }, url)
    }

    pub fn native_request(&mut self, url: Url) -> LoadTicket {
        let parent = self.current.as_ref().map(|d| d.visit);
        self.request(
            Intent::New {
                parent,
                source: None,
                origin: NewVisitOrigin::Page,
            },
            url,
        )
    }

    pub(crate) fn mark_form_post(&mut self, ticket: LoadTicket) {
        if let Pending::Loading {
            ticket: active,
            intent: Intent::New { origin, .. },
            ..
        } = &mut self.pending
        {
            if *active == ticket {
                *origin = NewVisitOrigin::FormPost;
            }
        }
    }

    pub fn redirected(&mut self, ticket: LoadTicket, actual: Url) {
        if let Pending::Loading {
            ticket: active,
            url,
            ..
        } = &mut self.pending
        {
            if *active == ticket {
                *url = actual;
            }
        }
    }

    pub fn failed(&mut self, ticket: LoadTicket) {
        if self.pending_ticket() == Some(ticket) {
            self.pending = Pending::Idle;
        }
    }

    /// Forgets the load in flight and the document, for a view left with no page.
    pub fn clear(&mut self) {
        self.pending = Pending::Idle;
        self.current = None;
    }

    pub fn valid(&self, document: &Document) -> bool {
        self.current.as_ref() == Some(document) && matches!(self.pending, Pending::Idle)
    }

    pub(crate) fn publish(&mut self, visit: VisitId, url: Url) {
        self.epoch += 1;
        self.current = Some(Document {
            view: self.view,
            visit,
            stored_url: url.clone(),
            page_url: url,
            epoch: self.epoch,
        });
        self.pending = Pending::Idle;
    }

    pub fn same_document(&mut self, page_url: Url) {
        if let Some(document) = &mut self.current {
            if document.page_url.origin() != page_url.origin() {
                return;
            }
            if document.page_url != page_url {
                self.epoch += 1;
                document.epoch = self.epoch;
                document.page_url = page_url;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_document_cannot_replace_the_committed_origin() {
        let mut navigation = Navigation::default();
        let committed = Url::parse("https://example.org/page").unwrap();
        navigation.publish(Ulid::new().to_string().parse().unwrap(), committed.clone());
        navigation.same_document(Url::parse("https://other.org/provisional").unwrap());
        assert_eq!(navigation.current.as_ref().unwrap().page_url, committed);
        let fragment = Url::parse("https://example.org/page#part").unwrap();
        navigation.same_document(fragment.clone());
        assert_eq!(navigation.current.as_ref().unwrap().page_url, fragment);
    }

    #[test]
    fn pending_target_names_the_page_before_it_commits() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.test/").unwrap();
        assert_eq!(navigation.pending_target(), None);
        let root = navigation.native_request(url.clone());
        assert_eq!(
            navigation.pending_target(),
            Some(PendingTarget::NewTree(url.clone()))
        );
        let visit: VisitId = Ulid::new().to_string().parse().unwrap();
        navigation.publish(visit, url.clone());
        assert_eq!(navigation.pending_target(), None);
        // A link from a committed page is a child, not a new tree.
        navigation.native_request(url.clone());
        assert_eq!(navigation.pending_target(), None);
        let other: VisitId = Ulid::new().to_string().parse().unwrap();
        let restore = navigation.request_queued(other, url);
        assert_eq!(
            navigation.pending_target(),
            Some(PendingTarget::Visit(other))
        );
        navigation.failed(restore);
        assert_eq!(navigation.pending_target(), None);
        assert!(!root.is_latest(&navigation));
    }

    #[test]
    fn completion_ticket_stays_identifiable_after_finish_but_not_after_another_load() {
        let mut navigation = Navigation::default();
        let url = Url::parse("https://example.test/").unwrap();
        let first = navigation.request(
            Intent::New {
                parent: None,
                source: None,
                origin: NewVisitOrigin::Explicit,
            },
            url.clone(),
        );
        navigation.started(first);
        navigation.failed(first);
        assert!(first.is_latest(&navigation));
        let second = navigation.request(
            Intent::New {
                parent: None,
                source: None,
                origin: NewVisitOrigin::Explicit,
            },
            url,
        );
        navigation.started(second);
        navigation.failed(second);
        assert!(navigation.pending_ticket().is_none());
        assert!(!first.is_latest(&navigation));
        assert!(second.is_latest(&navigation));
        assert!(!second.is_latest(&Navigation::default()));
    }
}
