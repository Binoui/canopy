use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use gtk::{cairo, gdk, gdk_pixbuf::Pixbuf};
use webkit2gtk::FaviconDatabaseExt;

const PAGE_LIMIT: usize = 512;
const HOST_LIMIT: usize = 256;
const STORED: i32 = 32;

type ChangeHandler = Box<dyn Fn(u64)>;

/// Page icons from WebKit's own favicon database, in a bounded cache keyed by page URL, as
/// PNG bytes for the chrome view. A queued page is never fetched: it borrows an icon already
/// seen on its host, or the view draws a monogram.
pub struct Favicons {
    database: webkit2gtk::FaviconDatabase,
    pages: RefCell<Bounded<String, Option<Rc<[u8]>>>>,
    hosts: RefCell<Bounded<String, Rc<[u8]>>>,
    requested: RefCell<HashSet<String>>,
    revision: Cell<u64>,
    on_change: RefCell<Option<ChangeHandler>>,
}

impl Favicons {
    pub fn new(database: &webkit2gtk::FaviconDatabase) -> Rc<Self> {
        let favicons = Rc::new(Self {
            database: database.clone(),
            pages: RefCell::new(Bounded::new(PAGE_LIMIT)),
            hosts: RefCell::new(Bounded::new(HOST_LIMIT)),
            requested: RefCell::new(HashSet::new()),
            revision: Cell::new(0),
            on_change: RefCell::new(None),
        });
        let weak = Rc::downgrade(&favicons);
        database.connect_favicon_changed(move |_, uri, _| {
            if let Some(favicons) = weak.upgrade() {
                favicons.requested.borrow_mut().remove(uri);
                favicons.request(uri.to_owned());
            }
        });
        favicons
    }

    /// Called with the new revision whenever a newly known icon could change what is drawn.
    pub fn connect_changed(&self, callback: impl Fn(u64) + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// The icon for `url` as PNG, or `None` for a monogram. An opened page missing from the
    /// cache is requested from the database, and `connect_changed` fires when it arrives.
    pub fn png(self: &Rc<Self>, url: &url::Url, opened: bool) -> Option<Rc<[u8]>> {
        let key = url.to_string();
        let known = self.pages.borrow_mut().get(&key).cloned();
        let icon = match known {
            Some(icon) => icon,
            None => {
                if opened {
                    self.request(key);
                }
                None
            }
        };
        icon.or_else(|| {
            let host = url.host_str()?.to_owned();
            self.hosts.borrow_mut().get(&host).cloned()
        })
    }

    fn request(self: &Rc<Self>, url: String) {
        if !self.requested.borrow_mut().insert(url.clone()) {
            return;
        }
        let weak = Rc::downgrade(self);
        self.database
            .favicon(&url.clone(), None::<&gio::Cancellable>, move |result| {
                if let Some(favicons) = weak.upgrade() {
                    favicons.store(url, result.ok().as_ref());
                }
            });
    }

    pub(crate) fn store(&self, url: String, surface: Option<&cairo::Surface>) {
        let png = surface.and_then(normalize);
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        if let (Some(host), Some(png)) = (host, &png) {
            self.hosts.borrow_mut().insert(host, png.clone());
        }
        let changed = png.is_some();
        self.pages.borrow_mut().insert(url, png);
        if changed {
            let revision = self.revision.get() + 1;
            self.revision.set(revision);
            if let Some(callback) = self.on_change.borrow().as_ref() {
                callback(revision);
            }
        }
    }
}

fn normalize(surface: &cairo::Surface) -> Option<Rc<[u8]>> {
    let image = cairo::ImageSurface::try_from(surface.clone()).ok()?;
    let pixbuf = gdk::pixbuf_get_from_surface(surface, 0, 0, image.width(), image.height())?;
    let pixbuf: Pixbuf =
        pixbuf.scale_simple(STORED, STORED, gtk::gdk_pixbuf::InterpType::Bilinear)?;
    pixbuf.save_to_bufferv("png", &[]).ok().map(Rc::from)
}

/// A map that forgets its least recently used entry beyond `limit`.
struct Bounded<K, V> {
    limit: usize,
    entries: HashMap<K, V>,
    order: VecDeque<K>,
}

impl<K: std::hash::Hash + Eq + Clone, V> Bounded<K, V> {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&mut self, key: &K) -> Option<&V> {
        if self.entries.contains_key(key) {
            self.touch(key);
        }
        self.entries.get(key)
    }

    fn insert(&mut self, key: K, value: V) {
        if self.entries.insert(key.clone(), value).is_some() {
            self.touch(&key);
        } else {
            self.order.push_back(key);
        }
        while self.order.len() > self.limit {
            if let Some(old) = self.order.pop_front() {
                self.entries.remove(&old);
            }
        }
    }

    fn touch(&mut self, key: &K) {
        if let Some(index) = self.order.iter().position(|k| k == key) {
            let key = self.order.remove(index).unwrap();
            self.order.push_back(key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Bounded;

    #[test]
    fn the_cache_forgets_the_least_recently_used_page() {
        let mut cache = Bounded::new(2);
        cache.insert("a", 1);
        cache.insert("b", 2);
        assert_eq!(cache.get(&"a"), Some(&1));
        cache.insert("c", 3);
        assert_eq!(cache.get(&"b"), None);
        assert_eq!(cache.get(&"a"), Some(&1));
        assert_eq!(cache.get(&"c"), Some(&3));
        cache.insert("a", 4);
        assert_eq!(cache.entries.len(), 2);
        assert_eq!(cache.get(&"a"), Some(&4));
    }
}
