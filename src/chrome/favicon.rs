use gtk::cairo;
use url::Url;

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

/// Icons for pages in the chrome.
pub(crate) struct Favicons;

impl Favicons {
    pub fn surface(&self, url: &Url, size: i32, scale: i32) -> cairo::Surface {
        monogram(url.host_str().unwrap_or("?"), size, scale)
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
