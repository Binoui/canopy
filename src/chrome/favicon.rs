use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

use gtk::{cairo, gdk, gdk::prelude::GdkPixbufExt, gdk_pixbuf::Pixbuf};
use webkit2gtk::{FaviconDatabaseExt, WebContextExt, WebViewExt};

use crate::browser::{Visit, VisitState};

const PAGE_LIMIT: usize = 512;
const HOST_LIMIT: usize = 256;
const STORED: i32 = 32;

const MONOGRAM_COLORS: [(f64, f64, f64); 8] = [
    (0.85, 0.36, 0.26),
    (0.23, 0.48, 0.84),
    (0.18, 0.62, 0.37),
    (0.61, 0.42, 0.87),
    (0.85, 0.59, 0.18),
    (0.23, 0.60, 0.68),
    (0.82, 0.33, 0.48),
    (0.42, 0.45, 0.50),
];

/// Page icons from WebKit's own favicon database, in a bounded cache keyed by page URL.
/// A queued page is never fetched: it borrows an icon already seen on its host, or gets
/// a monogram.
pub(crate) struct Favicons {
    page: webkit2gtk::WebView,
    pages: RefCell<Bounded<String, Option<Pixbuf>>>,
    hosts: RefCell<Bounded<String, Pixbuf>>,
    requested: RefCell<HashSet<String>>,
    revision: Cell<u64>,
    on_change: RefCell<Option<Box<dyn Fn()>>>,
}

impl Favicons {
    pub fn new(page: &webkit2gtk::WebView) -> Rc<Self> {
        let favicons = Rc::new(Self {
            page: page.clone(),
            pages: RefCell::new(Bounded::new(PAGE_LIMIT)),
            hosts: RefCell::new(Bounded::new(HOST_LIMIT)),
            requested: RefCell::new(HashSet::new()),
            revision: Cell::new(0),
            on_change: RefCell::new(None),
        });
        let weak = Rc::downgrade(&favicons);
        page.connect_favicon_notify(move |page| {
            let Some(favicons) = weak.upgrade() else {
                return;
            };
            if let (Some(uri), Some(surface)) = (page.uri(), page.favicon()) {
                favicons.store(uri.to_string(), Some(&surface));
            }
        });
        favicons
    }

    pub fn connect_changed(&self, callback: impl Fn() + 'static) {
        *self.on_change.borrow_mut() = Some(Box::new(callback));
    }

    /// Bumps whenever a newly known icon could change what is drawn.
    pub fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub fn surface(self: &Rc<Self>, visit: &Visit, size: i32, scale: i32) -> cairo::Surface {
        self.url_surface(&visit.url, visit.state == VisitState::Opened, size, scale)
    }

    pub fn url_surface(
        self: &Rc<Self>,
        url: &url::Url,
        opened: bool,
        size: i32,
        scale: i32,
    ) -> cairo::Surface {
        let key = url.to_string();
        let host = url.host_str().unwrap_or("?");
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
        let icon = icon.or_else(|| self.hosts.borrow_mut().get(&host.to_owned()).cloned());
        icon.and_then(|pixbuf| {
            pixbuf
                .scale_simple(
                    size * scale,
                    size * scale,
                    gtk::gdk_pixbuf::InterpType::Bilinear,
                )?
                .create_surface(scale, None::<&gdk::Window>)
        })
        .unwrap_or_else(|| monogram(host, size, scale))
    }

    fn request(self: &Rc<Self>, url: String) {
        if !self.requested.borrow_mut().insert(url.clone()) {
            return;
        }
        let Some(database) = self.page.context().and_then(|c| c.favicon_database()) else {
            return;
        };
        let weak = Rc::downgrade(self);
        database.favicon(&url.clone(), None::<&gio::Cancellable>, move |result| {
            if let Some(favicons) = weak.upgrade() {
                favicons.store(url, result.ok().as_ref());
            }
        });
    }

    fn store(&self, url: String, surface: Option<&cairo::Surface>) {
        let pixbuf = surface.and_then(normalize);
        let host = url::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_owned));
        if let (Some(host), Some(pixbuf)) = (host, &pixbuf) {
            self.hosts.borrow_mut().insert(host, pixbuf.clone());
        }
        let changed = pixbuf.is_some();
        self.pages.borrow_mut().insert(url, pixbuf);
        if changed {
            self.revision.set(self.revision.get() + 1);
            if let Some(callback) = self.on_change.borrow().as_ref() {
                callback();
            }
        }
    }
}

fn normalize(surface: &cairo::Surface) -> Option<Pixbuf> {
    let image = cairo::ImageSurface::try_from(surface.clone()).ok()?;
    let pixbuf = gdk::pixbuf_get_from_surface(surface, 0, 0, image.width(), image.height())?;
    pixbuf.scale_simple(STORED, STORED, gtk::gdk_pixbuf::InterpType::Bilinear)
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

/// The prototype's fallback: the host's initial on a color derived from the host.
fn monogram(host: &str, size: i32, scale: i32) -> cairo::Surface {
    let host = host.strip_prefix("www.").unwrap_or(host);
    let letter = host.chars().next().map_or('?', |c| c.to_ascii_uppercase());
    let hash = host.bytes().fold(2166136261u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(16777619)
    });
    let (r, g, b) = MONOGRAM_COLORS[hash as usize % MONOGRAM_COLORS.len()];
    let surface = cairo::ImageSurface::create(cairo::Format::ARgb32, size * scale, size * scale)
        .expect("a small image surface");
    surface.set_device_scale(f64::from(scale), f64::from(scale));
    let cr = cairo::Context::new(&surface).expect("a context for an image surface");
    let s = f64::from(size);
    let radius = 3.0f64.min(s / 4.);
    cr.new_sub_path();
    cr.arc(s - radius, radius, radius, -std::f64::consts::FRAC_PI_2, 0.);
    cr.arc(
        s - radius,
        s - radius,
        radius,
        0.,
        std::f64::consts::FRAC_PI_2,
    );
    cr.arc(
        radius,
        s - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        radius,
        radius,
        radius,
        std::f64::consts::PI,
        1.5 * std::f64::consts::PI,
    );
    cr.close_path();
    cr.set_source_rgb(r, g, b);
    let _ = cr.fill();
    cr.select_font_face(
        super::theme::FONT,
        cairo::FontSlant::Normal,
        cairo::FontWeight::Bold,
    );
    cr.set_font_size((s * 0.62).round());
    let text = letter.to_string();
    if let Ok(extents) = cr.text_extents(&text) {
        cr.move_to(
            (s - extents.width()) / 2. - extents.x_bearing(),
            (s - extents.height()) / 2. - extents.y_bearing(),
        );
        cr.set_source_rgb(1., 1., 1.);
        let _ = cr.show_text(&text);
    }
    drop(cr);
    (*surface).clone()
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
