use std::rc::Rc;

use anyhow::Result;
use gtk::prelude::*;

use super::Ui;
use crate::browser::{CaptureKind, VisitId};

pub(super) fn show_capture(ui: &Rc<Ui>, id: i64) -> Result<()> {
    let capture = ui.browser.borrow().capture(id)?;
    let visit = ui.browser.borrow().visit(capture.visit)?;
    let dialog = gtk::Dialog::with_buttons(
        Some("Saved text"),
        Some(&ui.window),
        gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Close", gtk::ResponseType::Close)],
    );
    dialog.set_widget_name("capture-reader");
    dialog.set_default_size(760, 620);
    let area = dialog.content_area();
    area.set_spacing(10);
    area.set_margin_start(18);
    area.set_margin_end(18);
    area.set_margin_top(14);
    area.set_margin_bottom(14);
    let heading = plain_label(&capture.title, 75);
    heading.style_context().add_class("capture-title");
    area.pack_start(&heading, false, false, 0);
    let kind = if capture.kind == CaptureKind::Selection {
        "Selection"
    } else {
        "Page"
    };
    area.pack_start(
        &plain_label(
            &format!(
                "{kind} remembered on {} · {}",
                date(capture.captured_at),
                capture.document_url
            ),
            90,
        ),
        false,
        false,
        0,
    );
    if let Some(source) = &capture.source {
        let label = plain_label(
            &format!(
                "From {}\n{}\n{}",
                source.page_url, source.link_text, source.excerpt
            ),
            85,
        );
        label.set_widget_name("capture-source");
        area.pack_start(&label, false, false, 0);
    }
    let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let live = gtk::Button::with_label("Open the page");
    let source = gtk::Button::with_label("Back to source");
    let citation = gtk::Button::with_label("Open source comment");
    source.set_sensitive(visit.parent.is_some());
    citation.set_sensitive(
        visit
            .source
            .as_ref()
            .and_then(|source| source.citation_url.as_ref())
            .is_some(),
    );
    actions.pack_start(&live, false, false, 0);
    actions.pack_start(&source, false, false, 0);
    actions.pack_start(&citation, false, false, 0);
    area.pack_start(&actions, false, false, 0);
    let text = gtk::TextView::new();
    text.set_widget_name("capture-text");
    text.set_editable(false);
    text.set_wrap_mode(gtk::WrapMode::WordChar);
    text.set_left_margin(10);
    text.set_right_margin(10);
    text.buffer().unwrap().set_text(&capture.text);
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Automatic);
    scroll.add(&text);
    area.pack_start(&scroll, true, true, 0);
    {
        let weak = Rc::downgrade(ui);
        let capture = capture.clone();
        let reader = dialog.downgrade();
        live.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                match ui.engine.open_saved(&capture) {
                    Ok(()) => {
                        if let Some(reader) = reader.upgrade() {
                            reader.close();
                        }
                    }
                    Err(error) => ui.show_error(&error.to_string()),
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(ui);
        let parent = visit.parent;
        let reader = dialog.downgrade();
        source.set_widget_name("capture-restore-source");
        source.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                if let Some(parent) = parent {
                    match ui.engine.restore(parent) {
                        Ok(()) => {
                            ui.search.set_text("");
                            if let Some(reader) = reader.upgrade() {
                                reader.close();
                            }
                        }
                        Err(error) => ui.show_error(&error.to_string()),
                    }
                }
            }
        });
    }
    {
        let weak = Rc::downgrade(ui);
        let child = visit.id;
        let reader = dialog.downgrade();
        citation.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                match ui.engine.open_citation(child) {
                    Ok(()) => {
                        ui.search.set_text("");
                        if let Some(reader) = reader.upgrade() {
                            reader.close();
                        }
                    }
                    Err(error) => ui.show_error(&error.to_string()),
                }
            }
        });
    }
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.show_all();
    Ok(())
}

pub(super) fn show_captures(ui: &Rc<Ui>, visit: VisitId) -> Result<()> {
    let captures = ui.browser.borrow().captures(visit)?;
    if captures.is_empty() {
        ui.show_error("No saved text for this page yet. Select some text and remember it, or turn on Remember pages.");
        return Ok(());
    }
    if captures.len() == 1 {
        return show_capture(ui, captures[0].id);
    }
    let dialog = gtk::Dialog::with_buttons(
        Some("Saved text for this page"),
        Some(&ui.window),
        gtk::DialogFlags::DESTROY_WITH_PARENT,
        &[("Close", gtk::ResponseType::Close)],
    );
    dialog.set_default_size(500, 420);
    let list = gtk::Box::new(gtk::Orientation::Vertical, 8);
    list.set_margin_start(16);
    list.set_margin_end(16);
    list.set_margin_top(12);
    list.set_margin_bottom(12);
    for capture in captures {
        let label = format!("{} · {}", date(capture.captured_at), capture.title);
        let button = gtk::Button::with_label(&label);
        let weak = Rc::downgrade(ui);
        button.connect_clicked(move |_| {
            if let Some(ui) = weak.upgrade() {
                ui.report(show_capture(&ui, capture.id));
            }
        });
        list.pack_start(&button, false, false, 0);
    }
    let scroll = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroll.add(&list);
    dialog.content_area().pack_start(&scroll, true, true, 0);
    dialog.connect_response(|dialog, _| dialog.close());
    dialog.show_all();
    Ok(())
}

pub(super) fn plain_label(text: &str, width: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_max_width_chars(width);
    label
}

pub(super) fn date(timestamp: i64) -> String {
    glib::DateTime::from_unix_local(timestamp / 1000)
        .and_then(|value| value.format("%d %b %Y, %H:%M"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "Unknown date".into())
}
