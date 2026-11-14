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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub view: ViewId,
    pub visit: VisitId,
    pub stored_url: Url,
    pub page_url: Url,
    pub epoch: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Visibility {
    Selected,
    Hidden,
}

#[derive(Clone, Debug)]
pub(super) enum Intent {
    New {
        parent: Option<VisitId>,
        source: Option<Box<super::SourceContext>>,
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

    pub fn native_request(&mut self, url: Url) -> LoadTicket {
        let parent = self.current.as_ref().map(|d| d.visit);
        self.request(
            Intent::New {
                parent,
                source: None,
            },
            url,
        )
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
            if document.page_url != page_url {
                self.epoch += 1;
                document.epoch = self.epoch;
                document.page_url = page_url;
            }
        }
    }
}
