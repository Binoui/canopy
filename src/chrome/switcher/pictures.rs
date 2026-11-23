use std::borrow::Cow;

use gtk::{cairo, gdk_pixbuf};
use wry::http::{header, Method, Request, Response};

use super::{BoxPx, Inner};
use crate::{browser::VisitId, pages::SwitcherSource};

pub(super) const MAX_IN_FLIGHT: usize = 4;
pub(super) const RAW_BUDGET: u64 = 36 * 1024 * 1024;

pub(super) enum Picture {
    Unavailable,
    Waiting(SwitcherSource),
    InFlight(gio::Cancellable),
    Ready(Pair),
}

impl Drop for Picture {
    fn drop(&mut self) {
        use gio::prelude::CancellableExt;
        if let Self::InFlight(cancel) = self {
            cancel.cancel();
        }
    }
}

pub(super) struct Pair {
    large: Option<Vec<u8>>,
    thumb: Vec<u8>,
}

pub(super) struct Pixels {
    bytes: Vec<u8>,
    width: i32,
    height: i32,
    stride: usize,
    alpha: bool,
}

#[derive(Clone, Copy)]
pub(super) struct Sizes {
    pub(super) large: Option<BoxPx>,
    pub(super) thumb: (i32, i32),
    pub(super) scale: i32,
}

impl Pixels {
    pub(super) fn copy(surface: cairo::Surface) -> Option<Self> {
        let image = cairo::ImageSurface::try_from(surface).ok()?;
        let width = image.width();
        let height = image.height();
        let stride = image.stride() as usize;
        let alpha = match image.format() {
            cairo::Format::ARgb32 => true,
            cairo::Format::Rgb24 => false,
            _ => return None,
        };
        let mut bytes = Vec::new();
        image.with_data(|data| bytes.extend_from_slice(data)).ok()?;
        Some(Self {
            bytes,
            width,
            height,
            stride,
            alpha,
        })
    }

    pub(super) fn encode(self, sizes: Sizes) -> Option<Pair> {
        let Self {
            bytes,
            width,
            height,
            stride,
            alpha,
        } = self;
        let mut rgb = Vec::with_capacity(width as usize * height as usize * 3);
        for row in bytes.chunks_exact(stride).take(height as usize) {
            for pixel in row[..width as usize * 4].chunks_exact(4) {
                let [a, r, g, b] = u32::from_ne_bytes(pixel.try_into().unwrap()).to_be_bytes();
                let white = if alpha { 255 - a } else { 0 };
                rgb.extend_from_slice(&[
                    r.saturating_add(white),
                    g.saturating_add(white),
                    b.saturating_add(white),
                ]);
            }
        }
        let pixbuf = gdk_pixbuf::Pixbuf::from_bytes(
            &glib::Bytes::from_owned(rgb),
            gdk_pixbuf::Colorspace::Rgb,
            false,
            8,
            width,
            height,
            width * 3,
        );
        let jpeg = |w, h| {
            let ratio = (f64::from(w * sizes.scale) / f64::from(width))
                .min(f64::from(h * sizes.scale) / f64::from(height))
                .min(1.);
            let w = (f64::from(width) * ratio).round().max(1.) as i32;
            let h = (f64::from(height) * ratio).round().max(1.) as i32;
            let scaled = pixbuf.scale_simple(w, h, gdk_pixbuf::InterpType::Bilinear)?;
            scaled.save_to_bufferv("jpeg", &[("quality", "88")]).ok()
        };
        Some(Pair {
            thumb: jpeg(sizes.thumb.0, sizes.thumb.1)?,
            large: match sizes.large {
                Some(BoxPx(_, _, w, h)) => Some(jpeg(w, h)?),
                None => None,
            },
        })
    }
}

pub(in crate::chrome) fn response(
    request: &Request<Vec<u8>>,
    inner: Option<&Inner>,
) -> Response<Cow<'static, [u8]>> {
    let bytes = inner.and_then(|inner| {
        if request.method() != Method::GET {
            return None;
        }
        let mut path = request
            .uri()
            .path()
            .strip_prefix("/switcher-image/")?
            .split('/');
        let id = path.next()?.parse().ok()?;
        let visit: VisitId = path.next()?.parse().ok()?;
        let variant = path.next()?;
        if path.next().is_some() {
            return None;
        }
        let switcher = inner.switcher.borrow();
        let opening = switcher.showing(id)?;
        let Picture::Ready(pair) = opening.pictures.get(&visit)? else {
            return None;
        };
        match variant {
            "thumb" => Some(pair.thumb.clone()),
            "large" => pair.large.clone(),
            _ => None,
        }
    });
    let mut response = Response::builder().header(header::CACHE_CONTROL, "no-store");
    if bytes.is_some() {
        response = response.header(header::CONTENT_TYPE, "image/jpeg");
    } else {
        response = response.status(404);
    }
    response
        .body(Cow::Owned(bytes.unwrap_or_default()))
        .unwrap()
}

impl Pair {
    pub(super) fn has_large(&self) -> bool {
        self.large.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_opening_picture_route_is_not_cacheable() {
        let id = ulid::Ulid::new();
        let request = Request::get(format!("canopy://localhost/switcher-image/1/{id}/thumb"))
            .body(vec![])
            .unwrap();
        let reply = response(&request, None);
        assert_eq!(reply.status(), 404);
        assert_eq!(reply.headers()[header::CACHE_CONTROL], "no-store");
        assert!(reply.body().is_empty());
    }
}
