use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::Path,
    rc::Rc,
};

use anyhow::Result;
use gtk::prelude::*;

use crate::{
    address::destination,
    browser::{Browser, CaptureKind, NavigationTarget, SearchHit, Snapshot, VisitId, VisitState},
    engine::{Engine, EngineEvent},
};

pub struct Ui {
    pub window: gtk::Window,
    pub engine: Engine,
    browser: Rc<RefCell<Browser>>,
    address: gtk::Entry,
    address_revealer: gtk::Revealer,
    navigation_target: Cell<NavigationTarget>,
    address_mode: gtk::Label,
    crumbs: gtk::Box,
    domain: gtk::Button,
    outline: gtk::TreeView,
    rows: gtk::TreeStore,
    search: gtk::SearchEntry,
    status: gtk::Label,
    notice: gtk::Label,
    back: gtk::Button,
    reload: gtk::Button,
    automatic: gtk::CheckButton,
    saved: gtk::Button,
    results: gtk::ListBox,
    navigation_stack: gtk::Stack,
    source_box: gtk::Box,
    source_label: gtk::Label,
    source_button: gtk::Button,
    citation_button: gtk::Button,
}

impl Ui {
    pub fn new(data_dir: &Path) -> Result<Rc<Self>> {
        let browser = Rc::new(RefCell::new(Browser::open(
            &data_dir.join("forest.sqlite3"),
        )?));
        let provider = gtk::CssProvider::new();
        provider.load_from_data(STYLE.as_bytes())?;
        if let Some(screen) = gdk::Screen::default() {
            gtk::StyleContext::add_provider_for_screen(
                &screen,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
        }
        let window = gtk::Window::new(gtk::WindowType::Toplevel);
        window.set_title("Canopy");
        window.set_default_size(1240, 800);
        window.set_widget_name("canopy");
        let shell = gtk::Box::new(gtk::Orientation::Vertical, 0);
        window.add(&shell);
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        toolbar.set_widget_name("toolbar");
        toolbar.set_margin_start(12);
        toolbar.set_margin_end(12);
        toolbar.set_margin_top(9);
        toolbar.set_margin_bottom(9);
        let back = icon_button("go-previous-symbolic", "Revenir au parent · Alt+←");
        let reload = icon_button("view-refresh-symbolic", "Recharger · Ctrl+R");
        toolbar.pack_start(&back, false, false, 0);
        toolbar.pack_start(&reload, false, false, 0);
        let crumbs = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        let crumb_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        crumb_scroll.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
        crumb_scroll.set_min_content_height(36);
        crumb_scroll.add(&crumbs);
        toolbar.pack_start(&crumb_scroll, true, true, 8);
        let domain = gtk::Button::with_label("Adresse ou recherche · Ctrl+L");
        domain.set_widget_name("domain");
        toolbar.pack_end(&domain, false, false, 0);
        shell.pack_start(&toolbar, false, false, 0);
        shell.pack_start(
            &gtk::Separator::new(gtk::Orientation::Horizontal),
            false,
            false,
            0,
        );

        let memory = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        memory.set_widget_name("memory-toolbar");
        memory.set_margin_start(14);
        memory.set_margin_end(14);
        memory.set_margin_top(7);
        memory.set_margin_bottom(7);
        let automatic = gtk::CheckButton::with_label("Mémoriser les pages");
        automatic.set_widget_name("automatic-capture");
        automatic.set_tooltip_text(Some("Conserver le texte des pages ouvertes sur cet ordinateur. Les champs de formulaire sont exclus."));
        automatic.set_active(browser.borrow().capture_enabled()?);
        let remember = gtk::Button::with_label("Mémoriser la sélection");
        remember.set_widget_name("remember-selection");
        remember.set_tooltip_text(Some(
            "Sélectionne le texte d'un post ou d'un article, puis mémorise-le · Ctrl+S",
        ));
        let saved = gtk::Button::with_label("Lire une capture");
        saved.set_widget_name("saved-captures");
        saved.set_tooltip_text(Some("Relire le texte enregistré, sans recharger le site."));
        memory.pack_start(&automatic, false, false, 0);
        memory.pack_start(&remember, false, false, 0);
        memory.pack_start(&saved, false, false, 0);
        shell.pack_start(&memory, false, false, 0);

        let address_revealer = gtk::Revealer::new();
        address_revealer.set_transition_type(gtk::RevealerTransitionType::None);
        let address_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        address_box.set_margin_start(16);
        address_box.set_margin_end(16);
        address_box.set_margin_top(10);
        address_box.set_margin_bottom(10);
        let address_mode = gtk::Label::new(Some("Depuis cette page"));
        let address = gtk::Entry::new();
        address.set_widget_name("address");
        address.set_placeholder_text(Some("Une adresse ou des mots à rechercher"));
        address.set_hexpand(true);
        address_box.pack_start(&address_mode, false, false, 0);
        address_box.pack_start(&address, true, true, 0);
        address_revealer.add(&address_box);
        shell.pack_start(&address_revealer, false, false, 0);
        let notice = gtk::Label::new(None);
        notice.set_widget_name("notice");
        notice.set_xalign(0.0);
        notice.set_line_wrap(true);
        notice.set_no_show_all(true);
        shell.pack_start(&notice, false, false, 0);

        let panels = gtk::Paned::new(gtk::Orientation::Horizontal);
        let sidebar = gtk::Box::new(gtk::Orientation::Vertical, 12);
        sidebar.set_widget_name("sidebar");
        sidebar.set_size_request(220, -1);
        let brand = gtk::Label::new(Some("canopy"));
        brand.set_widget_name("brand");
        brand.set_xalign(0.0);
        brand.set_margin_start(18);
        brand.set_margin_top(19);
        sidebar.pack_start(&brand, false, false, 0);
        let new_tree = gtk::Button::with_label("+  Nouvel arbre");
        new_tree.set_tooltip_text(Some("Démarrer un parcours indépendant · Ctrl+T"));
        new_tree.set_margin_start(12);
        new_tree.set_margin_end(12);
        sidebar.pack_start(&new_tree, false, false, 0);
        let search = gtk::SearchEntry::new();
        search.set_placeholder_text(Some("Texte, titre ou site…"));
        search.set_widget_name("memory-search");
        search.set_margin_start(12);
        search.set_margin_end(12);
        sidebar.pack_start(&search, false, false, 0);
        let heading = gtk::Label::new(Some("MES PARCOURS"));
        heading.set_widget_name("section-title");
        heading.set_xalign(0.0);
        heading.set_margin_start(18);
        sidebar.pack_start(&heading, false, false, 0);
        let rows = gtk::TreeStore::new(&[
            String::static_type(),
            String::static_type(),
            String::static_type(),
            i32::static_type(),
        ]);
        let outline = gtk::TreeView::with_model(&rows);
        outline.set_widget_name("outline");
        outline.set_headers_visible(false);
        outline.set_activate_on_single_click(true);
        outline.set_tooltip_column(2);
        let renderer = gtk::CellRendererText::new();
        renderer.set_ellipsize(gtk::pango::EllipsizeMode::End);
        let column = gtk::TreeViewColumn::new();
        gtk::prelude::TreeViewColumnExt::pack_start(&column, &renderer, true);
        gtk::prelude::TreeViewColumnExt::add_attribute(&column, &renderer, "text", 1);
        gtk::prelude::TreeViewColumnExt::add_attribute(&column, &renderer, "weight", 3);
        column.set_expand(true);
        outline.append_column(&column);
        let outline_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        outline_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        outline_scroll.add(&outline);
        let results = gtk::ListBox::new();
        results.set_widget_name("search-results");
        results.set_selection_mode(gtk::SelectionMode::None);
        let result_scroll =
            gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
        result_scroll.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
        result_scroll.add(&results);
        let navigation_stack = gtk::Stack::new();
        navigation_stack.set_widget_name("navigation-mode");
        navigation_stack.add_named(&outline_scroll, "tree");
        navigation_stack.add_named(&result_scroll, "search");
        sidebar.pack_start(&navigation_stack, true, true, 0);
        let status = gtk::Label::new(None);
        status.set_xalign(0.0);
        status.set_margin_start(18);
        status.set_margin_bottom(14);
        status.set_widget_name("status");
        sidebar.pack_end(&status, false, false, 0);
        panels.pack1(&sidebar, false, false);
        panels.set_position(265);
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let source_box = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        source_box.set_widget_name("source-context");
        source_box.set_no_show_all(true);
        source_box.set_margin_start(16);
        source_box.set_margin_end(16);
        source_box.set_margin_top(8);
        source_box.set_margin_bottom(8);
        let source_label = gtk::Label::new(None);
        source_label.set_xalign(0.0);
        source_label.set_line_wrap(true);
        source_label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        source_label.set_lines(3);
        source_label.set_max_width_chars(75);
        let source_button = gtk::Button::with_label("Source");
        source_button.set_widget_name("restore-source");
        let citation_button = gtk::Button::with_label("Ouvrir le commentaire");
        citation_button.set_widget_name("open-citation");
        source_box.pack_start(&source_label, true, true, 0);
        source_box.pack_start(&source_button, false, false, 0);
        source_box.pack_start(&citation_button, false, false, 0);
        content.pack_start(&source_box, false, false, 0);
        let engine = Engine::new(&content, browser.clone(), &data_dir.join("profile"))?;
        panels.pack2(&content, true, false);
        shell.pack_start(&panels, true, true, 0);
        let ui = Rc::new(Self {
            window,
            engine,
            browser,
            address,
            address_revealer,
            navigation_target: Cell::new(NavigationTarget::Root),
            address_mode,
            crumbs,
            domain,
            outline,
            rows,
            search,
            status,
            notice,
            back,
            reload,
            automatic,
            saved,
            results,
            navigation_stack,
            source_box,
            source_label,
            source_button,
            citation_button,
        });
        {
            let weak = Rc::downgrade(&ui);
            ui.engine.observe(move |event| {
                if let Some(ui) = weak.upgrade() {
                    match event {
                        EngineEvent::Changed => ui.render_or_notice(),
                        EngineEvent::Loading(true) => {
                            ui.notice.hide();
                            ui.status.set_text("Chargement…");
                        }
                        EngineEvent::Loading(false) => ui.render_or_notice(),
                        EngineEvent::Error(message) => ui.show_error(&message),
                        EngineEvent::Remembered(_) => {
                            ui.status.set_text("Sélection mémorisée localement.")
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.automatic.connect_toggled(move |button| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.engine.set_capture_enabled(button.is_active()));
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            remember.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.engine.remember_selection());
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.saved.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Ok(snapshot) = ui.snapshot() {
                        if let Some(id) = snapshot.current {
                            ui.report(ui.show_captures(id));
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.source_button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Ok(snapshot) = ui.snapshot() {
                        if let Some(parent) =
                            snapshot.current_visit().and_then(|visit| visit.parent)
                        {
                            match ui.engine.restore(parent) {
                                Ok(()) => ui.search.set_text(""),
                                Err(error) => ui.show_error(&error.to_string()),
                            }
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.citation_button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Ok(snapshot) = ui.snapshot() {
                        if let Some(child) = snapshot.current {
                            match ui.engine.open_citation(child) {
                                Ok(()) => ui.search.set_text(""),
                                Err(error) => ui.show_error(&error.to_string()),
                            }
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.address.connect_activate(move |entry| {
                if let Some(ui) = weak.upgrade() {
                    match destination(entry.text().as_str())
                        .and_then(|url| ui.engine.navigate(url, ui.navigation_target.get()))
                    {
                        Ok(()) => {
                            ui.hide_address();
                            ui.engine.native.grab_focus();
                        }
                        Err(error) => ui.show_error(&error.to_string()),
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.domain.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_current_address();
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            new_tree.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.open_address(NavigationTarget::Root);
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.back.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.engine.back());
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.reload.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.engine.reload());
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.search.connect_search_changed(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.render_or_notice();
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.outline.connect_row_activated(move |tree, path, _| {
                if let Some(ui) = weak.upgrade() {
                    if let Some((model, iter)) = tree
                        .model()
                        .and_then(|model| model.iter(path).map(|iter| (model, iter)))
                    {
                        if let Ok(id) = model.value(&iter, 0).get::<String>() {
                            if let Ok(id) = id.parse::<VisitId>() {
                                ui.report(ui.engine.restore(id));
                            }
                        }
                    }
                }
            });
        }
        {
            let weak = Rc::downgrade(&ui);
            ui.window.connect_key_press_event(move |_, event| {
                let Some(ui) = weak.upgrade() else {
                    return glib::Propagation::Proceed;
                };
                let key = event.keyval();
                let control = event.state().contains(gdk::ModifierType::CONTROL_MASK);
                let alt = event.state().contains(gdk::ModifierType::MOD1_MASK);
                if control && (key == gdk::keys::constants::l || key == gdk::keys::constants::L) {
                    ui.open_current_address();
                } else if control
                    && (key == gdk::keys::constants::t || key == gdk::keys::constants::T)
                {
                    ui.open_address(NavigationTarget::Root);
                } else if control
                    && (key == gdk::keys::constants::r || key == gdk::keys::constants::R)
                {
                    ui.report(ui.engine.reload());
                } else if control
                    && (key == gdk::keys::constants::s || key == gdk::keys::constants::S)
                {
                    ui.report(ui.engine.remember_selection());
                } else if control
                    && (key == gdk::keys::constants::q || key == gdk::keys::constants::Q)
                {
                    ui.window.close();
                } else if alt && key == gdk::keys::constants::Left {
                    ui.report(ui.engine.back());
                } else if key == gdk::keys::constants::Escape && ui.address_revealer.reveals_child()
                {
                    ui.hide_address();
                    ui.engine.native.grab_focus();
                } else {
                    return glib::Propagation::Proceed;
                }
                glib::Propagation::Stop
            });
        }
        ui.render()?;
        Ok(ui)
    }

    pub fn snapshot(&self) -> Result<Snapshot> {
        self.browser.borrow().snapshot()
    }

    pub fn resume(&self) -> Result<()> {
        self.engine.reload()
    }

    pub fn hide_address(&self) {
        self.address_revealer.set_reveal_child(false);
    }

    fn open_current_address(&self) {
        match self.snapshot() {
            Ok(snapshot) => self.open_address(
                snapshot
                    .current
                    .map_or(NavigationTarget::Root, NavigationTarget::ChildOf),
            ),
            Err(error) => self.show_error(&error.to_string()),
        }
    }

    pub fn open_address(&self, target: NavigationTarget) {
        self.navigation_target.set(target);
        let new_root = matches!(target, NavigationTarget::Root);
        self.address_mode.set_text(if new_root {
            "Nouvel arbre"
        } else {
            "Depuis cette page"
        });
        let current_url = self
            .snapshot()
            .ok()
            .and_then(|snapshot| snapshot.current_visit().map(|v| v.url.to_string()));
        self.address.set_text(if new_root {
            ""
        } else {
            current_url.as_deref().unwrap_or("")
        });
        self.address_revealer.set_reveal_child(true);
        self.address.grab_focus();
        self.address.select_region(0, -1);
    }

    fn render_or_notice(self: &Rc<Self>) {
        self.report(self.render());
    }

    fn report(&self, result: Result<()>) {
        if let Err(error) = result {
            self.show_error(&error.to_string());
        }
    }

    fn show_error(&self, message: &str) {
        self.notice.set_text(message);
        self.notice.show();
    }

    fn render_results(self: &Rc<Self>, hits: &[SearchHit]) {
        for child in self.results.children() {
            self.results.remove(&child);
        }
        if hits.is_empty() {
            let label = gtk::Label::new(Some("Aucune capture ni page correspondante."));
            label.set_line_wrap(true);
            label.set_margin_start(12);
            label.set_margin_end(12);
            label.set_margin_top(16);
            self.results.add(&label);
        }
        for hit in hits {
            let button = gtk::Button::new();
            button.set_widget_name("search-hit");
            let content = gtk::Box::new(gtk::Orientation::Vertical, 5);
            let title = plain_label(&hit.title, 32);
            title.style_context().add_class("result-title");
            content.pack_start(&title, false, false, 0);
            let detail = if let Some(at) = hit.captured_at {
                format!("Capture · {}", date(at))
            } else if hit.state == VisitState::Queued {
                "Lien en attente".into()
            } else {
                "Page visitée".into()
            };
            let label = plain_label(&detail, 32);
            label.style_context().add_class("result-detail");
            content.pack_start(&label, false, false, 0);
            let excerpt: String = hit.excerpt.chars().take(160).collect();
            let label = plain_label(&excerpt, 32);
            content.pack_start(&label, false, false, 0);
            if let Some(source) = &hit.source {
                let label = plain_label(
                    &format!(
                        "Depuis {}",
                        source.page_url.host_str().unwrap_or("la source")
                    ),
                    32,
                );
                label.style_context().add_class("result-detail");
                content.pack_start(&label, false, false, 0);
            }
            button.add(&content);
            let weak = Rc::downgrade(self);
            let visit = hit.visit;
            let capture = hit.capture_id;
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    if let Some(id) = capture {
                        ui.report(ui.show_capture(id));
                    } else {
                        ui.report(ui.engine.restore(visit));
                    }
                }
            });
            self.results.add(&button);
        }
        self.results.show_all();
    }

    pub fn search_hits(&self, query: &str) -> Result<Vec<SearchHit>> {
        self.browser.borrow().search(query)
    }

    pub fn show_capture(self: &Rc<Self>, id: i64) -> Result<()> {
        let capture = self.browser.borrow().capture(id)?;
        let visit = self.browser.borrow().visit(capture.visit)?;
        let dialog = gtk::Dialog::with_buttons(
            Some("Capture locale"),
            Some(&self.window),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[("Fermer", gtk::ResponseType::Close)],
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
            "Sélection"
        } else {
            "Page"
        };
        area.pack_start(
            &plain_label(
                &format!(
                    "{kind} mémorisée le {} · {}",
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
                    "Depuis {}\n{}\n{}",
                    source.page_url, source.link_text, source.excerpt
                ),
                85,
            );
            label.set_widget_name("capture-source");
            area.pack_start(&label, false, false, 0);
        }
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let live = gtk::Button::with_label("Ouvrir la page");
        let source = gtk::Button::with_label("Revenir à la source");
        let citation = gtk::Button::with_label("Ouvrir le commentaire source");
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
            let weak = Rc::downgrade(self);
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
            let weak = Rc::downgrade(self);
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
            let weak = Rc::downgrade(self);
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

    fn show_captures(self: &Rc<Self>, visit: VisitId) -> Result<()> {
        let captures = self.browser.borrow().captures(visit)?;
        if captures.is_empty() {
            self.show_error("Aucune capture pour cette page. Sélectionne du texte et mémorise-le, ou active la mémorisation des pages.");
            return Ok(());
        }
        if captures.len() == 1 {
            return self.show_capture(captures[0].id);
        }
        let dialog = gtk::Dialog::with_buttons(
            Some("Captures de cette page"),
            Some(&self.window),
            gtk::DialogFlags::DESTROY_WITH_PARENT,
            &[("Fermer", gtk::ResponseType::Close)],
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
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.show_capture(capture.id));
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

    fn render(self: &Rc<Self>) -> Result<()> {
        let snapshot = self.snapshot()?;
        let current = snapshot.current_visit();
        self.back
            .set_sensitive(current.is_some_and(|visit| visit.parent.is_some()));
        self.reload.set_sensitive(current.is_some());
        if let Some(visit) = current {
            self.window
                .set_title(&format!("{} · Canopy", visit.label()));
            self.domain
                .set_label(visit.url.host_str().unwrap_or("Adresse"));
            self.domain.set_tooltip_text(Some(visit.url.as_str()));
        } else {
            self.window.set_title("Canopy");
            self.domain.set_label("Adresse ou recherche · Ctrl+L");
        }
        for child in self.crumbs.children() {
            self.crumbs.remove(&child);
        }
        for (index, visit) in snapshot.breadcrumb().iter().enumerate() {
            if index > 0 {
                self.crumbs
                    .pack_start(&gtk::Label::new(Some("›")), false, false, 2);
            }
            let label: String = visit.label().chars().take(26).collect();
            let button = gtk::Button::with_label(&label);
            button.style_context().add_class("crumb");
            button.set_tooltip_text(Some(visit.url.as_str()));
            let id = visit.id;
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(ui) = weak.upgrade() {
                    ui.report(ui.engine.restore(id));
                }
            });
            self.crumbs.pack_start(&button, false, false, 0);
        }
        self.crumbs.show_all();
        self.saved.set_sensitive(current.is_some());
        if let Some(source) = current.and_then(|visit| visit.source.as_ref()) {
            let excerpt: String = source.excerpt.chars().take(240).collect();
            self.source_label.set_text(&format!(
                "Depuis {} · {}\n{}",
                source.page_url.host_str().unwrap_or("la source"),
                source.link_text,
                excerpt
            ));
            self.source_label.set_tooltip_text(Some(&source.excerpt));
            self.source_button
                .set_sensitive(current.is_some_and(|visit| visit.parent.is_some()));
            self.citation_button
                .set_sensitive(source.citation_url.is_some());
            for child in self.source_box.children() {
                child.show_all();
            }
            self.source_box.show();
        } else {
            self.source_box.hide();
        }
        let query = self.search.text();
        let hits = if query.is_empty() {
            Vec::new()
        } else {
            self.browser.borrow().search(query.as_str())?
        };
        self.render_results(&hits);
        self.navigation_stack
            .set_visible_child_name(if query.is_empty() { "tree" } else { "search" });
        let shown = if query.is_empty() {
            None
        } else {
            let mut ids = self
                .browser
                .borrow()
                .search(query.as_str())?
                .into_iter()
                .map(|hit| hit.visit)
                .collect::<HashSet<_>>();
            let mut parents = ids.iter().copied().collect::<Vec<_>>();
            while let Some(id) = parents.pop() {
                if let Some(parent) = snapshot
                    .visits
                    .iter()
                    .find(|v| v.id == id)
                    .and_then(|v| v.parent)
                {
                    if ids.insert(parent) {
                        parents.push(parent);
                    }
                }
            }
            Some(ids)
        };
        self.rows.clear();
        let mut iters = HashMap::new();
        let mut selected = None;
        for visit in &snapshot.visits {
            if shown.as_ref().is_some_and(|ids| !ids.contains(&visit.id)) {
                continue;
            }
            let parent = visit.parent.and_then(|id| iters.get(&id));
            let iter = self.rows.append(parent);
            let marker = if visit.state == VisitState::Queued {
                "○ En attente · "
            } else if snapshot.current == Some(visit.id) {
                "● "
            } else if visit.parent.is_none() {
                "◦ "
            } else {
                ""
            };
            let label = format!("{marker}{}", visit.label());
            let weight = if snapshot.current == Some(visit.id) {
                700i32
            } else {
                400i32
            };
            self.rows.set(
                &iter,
                &[
                    (0, &visit.id.to_string()),
                    (1, &label),
                    (2, &visit.url.as_str()),
                    (3, &weight),
                ],
            );
            if snapshot.current == Some(visit.id) {
                selected = Some(iter);
            }
            iters.insert(visit.id, iter);
        }
        self.outline.expand_all();
        if let Some(iter) = selected {
            self.outline.selection().select_iter(&iter);
        }
        let roots = snapshot
            .visits
            .iter()
            .filter(|visit| visit.parent.is_none())
            .count();
        let queued = snapshot
            .visits
            .iter()
            .filter(|visit| visit.state == VisitState::Queued)
            .count();
        self.status.set_text(&format!(
            "{roots} arbre{} · {} page{} · {queued} en attente",
            if roots == 1 { "" } else { "s" },
            snapshot.visits.len(),
            if snapshot.visits.len() == 1 { "" } else { "s" }
        ));
        Ok(())
    }
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::from_icon_name(Some(icon), gtk::IconSize::Button);
    button.set_tooltip_text(Some(tooltip));
    button
}

fn plain_label(text: &str, width: i32) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.set_xalign(0.0);
    label.set_line_wrap(true);
    label.set_max_width_chars(width);
    label
}

fn date(timestamp: i64) -> String {
    glib::DateTime::from_unix_local(timestamp / 1000)
        .and_then(|value| value.format("%d %b %Y, %H:%M"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "Date indisponible".into())
}

const STYLE: &str = "
@define-color placeholder_text_color #778471;
#canopy { background: #f6f5f0; color: #29362e; }
#toolbar button { border: 0; background: transparent; box-shadow: none; padding: 5px 9px; color: #35513d; }
#toolbar label, #toolbar button label { color: #35513d; }
#toolbar button:disabled { color: #aab6a5; }
#toolbar button:hover { background: #e8ede5; }
#domain { color: #4c6954; font-size: 12px; }
#sidebar { background: #e9ede6; border-right: 1px solid #d6dfd2; }
#brand { font-size: 25px; font-weight: bold; color: #35513d; letter-spacing: -1px; }
#section-title { color: #798675; font-size: 10px; font-weight: bold; letter-spacing: 1px; }
#sidebar button { background: #f8faf5; border: 1px solid #cbd8c7; border-radius: 7px; box-shadow: none; padding: 9px; color: #35513d; }
#sidebar entry { border: 1px solid #d4ded0; background: #f4f7f0; border-radius: 7px; box-shadow: none; font-size: 12px; color: #3e5243; }
#sidebar entry image { color: #6f7f6d; }
#outline { background: #e9ede6; color: #3e5243; padding: 4px; }
#outline:selected { background: #cbdcc3; color: #24482e; }
#outline.view { font-size: 13px; }
#outline.view cell { padding: 8px 3px; }
#status { font-size: 11px; color: #778471; }
#address { border: 1px solid #6f9474; border-radius: 7px; box-shadow: none; padding: 7px; background: #ffffff; color: #29362e; }
#memory-toolbar { color: #3e5243; }
#memory-toolbar label, #memory-toolbar button label { color: #3e5243; }
#memory-toolbar button { color: #35513d; background: #edf2e9; border: 1px solid #ccd8c5; box-shadow: none; border-radius: 6px; }
#source-context { color: #526b55; }
#source-context label, #source-context button label { color: #526b55; }
#source-context button { color: #35513d; background: #eef3ea; box-shadow: none; border-radius: 6px; }
#search-results { background: #e9ede6; }
#search-results button { border: 0; background: transparent; padding: 12px; border-radius: 0; border-bottom: 1px solid #d4ded0; }
#search-results button:hover { background: #dce8d6; }
#search-results label { color: #354c3b; font-size: 12px; }
#search-results .result-title { font-weight: bold; font-size: 13px; }
#search-results .result-detail { color: #6a7c65; font-size: 10px; }
#capture-reader textview text { color: #29362e; background: #fffef9; }
#capture-reader label { color: #35513d; }
#capture-reader { background: #f6f5f0; }
#capture-reader button { color: #35513d; background: #edf2e9; border: 1px solid #ccd8c5; box-shadow: none; border-radius: 6px; }
#capture-reader .capture-title { font-size: 20px; font-weight: bold; }
#notice { color: #804c26; background: #fff0d9; padding: 10px 18px; }
";
