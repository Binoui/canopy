use std::f64::consts::PI;

use gtk::{cairo, prelude::*};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Icon {
    Plus,
    Search,
    Spark,
    Forest,
    Chevron,
    Globe,
}

/// A drawing of one of the prototype's icons in the widget's CSS color, so hover and
/// insensitive states follow the theme.
pub(crate) fn icon(icon: Icon, size: i32) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.set_size_request(size, size);
    area.set_valign(gtk::Align::Center);
    area.set_halign(gtk::Align::Center);
    area.connect_draw(move |area, cr| {
        let color = area.style_context().color(area.state_flags());
        cr.set_source_rgba(color.red(), color.green(), color.blue(), color.alpha());
        let scale = f64::from(area.allocated_width().min(area.allocated_height())) / 24.;
        cr.scale(scale, scale);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);
        draw(cr, icon);
        glib::Propagation::Stop
    });
    area
}

fn draw(cr: &cairo::Context, icon: Icon) {
    let stroke = |width: f64| {
        cr.set_line_width(width);
        let _ = cr.stroke();
    };
    let circle = |x: f64, y: f64, r: f64| {
        cr.new_sub_path();
        cr.arc(x, y, r, 0., 2. * PI);
    };
    match icon {
        Icon::Plus => {
            cr.move_to(12., 5.);
            cr.line_to(12., 19.);
            cr.move_to(5., 12.);
            cr.line_to(19., 12.);
            stroke(1.9);
        }
        Icon::Search => {
            circle(11., 11., 6.5);
            cr.move_to(20., 20.);
            cr.line_to(15.8, 15.8);
            stroke(1.9);
        }
        Icon::Spark => {
            star(cr, 11., 10., 7.5, 1.9);
            let _ = cr.fill();
            cr.push_group();
            star(cr, 18.5, 17.5, 3.5, 0.9);
            let _ = cr.fill();
            let _ = cr.pop_group_to_source();
            let _ = cr.paint_with_alpha(0.7);
        }
        Icon::Forest => {
            for (x, y) in [(4.5, 12.), (19.5, 5.5), (19.5, 12.), (19.5, 18.5)] {
                circle(x, y, 2.);
            }
            cr.move_to(6.5, 12.);
            cr.line_to(17.5, 12.);
            cr.move_to(10., 12.);
            cr.curve_to(12.5, 12., 13., 5.5, 17.5, 5.5);
            cr.move_to(10., 12.);
            cr.curve_to(12.5, 12., 13., 18.5, 17.5, 18.5);
            stroke(1.8);
        }
        Icon::Chevron => {
            cr.move_to(7., 10.);
            cr.line_to(12., 15.);
            cr.line_to(17., 10.);
            stroke(2.);
        }
        Icon::Globe => {
            circle(12., 12., 8.5);
            cr.move_to(3.5, 12.);
            cr.line_to(20.5, 12.);
            cr.move_to(12., 3.5);
            cr.curve_to(14.6, 6.1, 14.6, 17.9, 12., 20.5);
            cr.move_to(12., 3.5);
            cr.curve_to(9.4, 6.1, 9.4, 17.9, 12., 20.5);
            stroke(1.7);
        }
    }
}

/// The prototype's four-pointed spark: `r` is the tip distance, `k` the waist.
fn star(cr: &cairo::Context, x: f64, y: f64, r: f64, k: f64) {
    cr.move_to(x, y - r);
    cr.line_to(x + k, y - k);
    cr.line_to(x + r, y);
    cr.line_to(x + k, y + k);
    cr.line_to(x, y + r);
    cr.line_to(x - k, y + k);
    cr.line_to(x - r, y);
    cr.line_to(x - k, y - k);
    cr.close_path();
}
