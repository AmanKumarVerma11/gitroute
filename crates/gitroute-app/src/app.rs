//! The settings window: four tabs over the same config the CLI uses, plus the
//! install dialog. Everything that changes state goes through `Model`.

use std::fmt::Display;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use anyhow::{Context as _, Result, anyhow, bail};
use eframe::egui::{
    self, Align, Align2, Button, Color32, ComboBox, CornerRadius, FontId, Frame, Grid, Label,
    Layout, Margin, RichText, ScrollArea, Sense, Stroke, TextEdit, pos2, vec2,
};
use gitroute_core::accounts::{self, Added};
use gitroute_core::config::{Config, config_path, lock_dir};
use gitroute_core::github::{self, User};
use gitroute_core::helper_log::{self, Entry};
use gitroute_core::managed::{self, Change};
use gitroute_core::token_store::{Access, TokenStore, unix_now};
use gitroute_core::{paths, resolve};
use tray_icon::menu::MenuEvent;

use crate::glass::{self, Glass};
use crate::login::{self, Login};
use crate::theme;
use crate::tray::{Action, Tray};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Accounts,
    Rules,
    Check,
    Activity,
    Setup,
}

const TABS: [(Tab, &str); 5] = [
    (Tab::Accounts, "Accounts"),
    (Tab::Rules, "Folder rules"),
    (Tab::Check, "Check a path"),
    (Tab::Activity, "Activity"),
    (Tab::Setup, "Setup"),
];

/// How gitroute is connected to git.
enum SetupState {
    NotInstalled,
    /// Installed, but git points at a helper other than this app's (the CLI's, say).
    OtherHelper(Option<PathBuf>),
    Installed,
}

/// The config and the files gitroute owns, kept in step.
pub struct Model {
    pub home: PathBuf,
    pub global_gitconfig: PathBuf,
    pub store: TokenStore,
    /// The helper next to this binary; `install` points git at it.
    pub install_helper: Option<PathBuf>,
    pub config: Config,
    pub installed: bool,
    config_mtime: Option<SystemTime>,
}

impl Model {
    pub fn load(
        home: PathBuf,
        global_gitconfig: PathBuf,
        store: TokenStore,
        install_helper: Option<PathBuf>,
    ) -> Result<Model> {
        let mut model = Model {
            home,
            global_gitconfig,
            store,
            install_helper,
            config: Config::default(),
            installed: false,
            config_mtime: None,
        };
        model.reload()?;
        Ok(model)
    }

    fn config_file(&self) -> PathBuf {
        config_path(&self.home)
    }

    fn mtime(&self) -> Option<SystemTime> {
        fs::metadata(self.config_file())
            .and_then(|m| m.modified())
            .ok()
    }

    fn reload(&mut self) -> Result<()> {
        self.config = Config::load(&self.config_file())?;
        self.config_mtime = self.mtime();
        self.installed = managed::is_installed(&self.global_gitconfig)?;
        Ok(())
    }

    /// Picks up changes made by the CLI or the credential helper (such as a
    /// "needs re-login" flag). Returns true if anything changed.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let installed = managed::is_installed(&self.global_gitconfig)?;
        if self.mtime() == self.config_mtime && installed == self.installed {
            return Ok(false);
        }
        self.reload()?;
        Ok(true)
    }

    /// Saves the config and, once installed, regenerates the files git reads.
    pub fn save(&mut self) -> Result<()> {
        self.config.save(&self.config_file())?;
        self.config_mtime = self.mtime();
        if self.installed {
            managed::write_files(
                &self.config,
                &self.home,
                &managed::helper_for(&self.config)?,
            )?;
        }
        Ok(())
    }

    fn login_of(&self, id: u64) -> String {
        self.config
            .account(id)
            .map_or_else(|| format!("account {id} (removed)"), |a| a.login.clone())
    }

    /// Each account's line, in config order: (id, letter, color). Letters
    /// come from the login and never repeat, so lines differ in more than
    /// color.
    fn lines(&self, dark: bool) -> Vec<(u64, char, Color32)> {
        let mut used = Vec::new();
        self.config
            .accounts
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let letter = a
                    .login
                    .chars()
                    .filter(char::is_ascii_alphanumeric)
                    .map(|c| c.to_ascii_uppercase())
                    .find(|c| !used.contains(c))
                    .unwrap_or('?');
                used.push(letter);
                (a.id, letter, theme::line_color(i, dark))
            })
            .collect()
    }

    fn line(&self, id: u64, dark: bool) -> (char, Color32) {
        self.lines(dark)
            .into_iter()
            .find(|l| l.0 == id)
            .map_or(('?', theme::LINE_UNKNOWN), |l| (l.1, l.2))
    }

    /// `~/x` for paths under home.
    fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// An account id and its verified emails, or why they couldn't be loaded.
type EmailsLoaded = (u64, Result<Vec<String>, String>);

/// An install or uninstall waiting for the user to confirm the diff.
struct Pending {
    install: bool,
    change: Change,
}

enum LoginIntent {
    Start,
    Cancel,
    Close,
}

pub struct App {
    pub model: Model,
    pub tab: Tab,
    tray: Option<Tray>,
    menu_rx: Option<Receiver<MenuEvent>>,
    pub login: Option<Login>,
    emails_rx: Option<Receiver<EmailsLoaded>>,
    pending: Option<Pending>,
    confirm_remove: Option<u64>,
    check_input: String,
    activity: Vec<Entry>,
    activity_read: Option<Instant>,
    /// Text and whether it's an error.
    pub message: Option<(String, bool)>,
    want_visible: bool,
    last_reload: Instant,
    themed: bool,
    /// Native glass behind the sidebar; `None` in tests and on failure, where
    /// the sidebar is painted solid instead.
    glass: Option<Glass>,
}

impl App {
    pub fn new(model: Model, tray: Option<Tray>, menu_rx: Option<Receiver<MenuEvent>>) -> App {
        App {
            model,
            tab: Tab::Accounts,
            tray,
            menu_rx,
            login: None,
            emails_rx: None,
            pending: None,
            confirm_remove: None,
            check_input: String::new(),
            activity: Vec::new(),
            activity_read: None,
            message: None,
            want_visible: false,
            last_reload: Instant::now(),
            themed: false,
            glass: None,
        }
    }

    pub fn with_glass(mut self, glass: Option<Glass>) -> App {
        self.glass = glass;
        self
    }

    fn setup_state(&self) -> SetupState {
        if !self.model.installed {
            return SetupState::NotInstalled;
        }
        let current = managed::current_helper(&self.model.home);
        match &self.model.install_helper {
            Some(ours) if current.as_ref() != Some(ours) => SetupState::OtherHelper(current),
            _ => SetupState::Installed,
        }
    }

    fn info(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), false));
    }

    fn error(&mut self, e: impl Display) {
        self.message = Some((format!("{e:#}"), true));
    }

    /// Saves, refreshes the menu, and reports the outcome.
    fn save(&mut self, done: impl Into<String>) {
        match self.model.save() {
            Ok(()) => {
                self.info(done);
                self.config_changed();
            }
            Err(e) => self.error(e),
        }
    }

    fn config_changed(&mut self) {
        if let Some(tray) = &mut self.tray
            && let Err(e) = tray.update(&self.model.config)
        {
            self.error(e);
        }
    }

    /// Menu clicks, background results, and config changes made elsewhere.
    /// Runs every frame, and while the window is hidden.
    fn process(&mut self) {
        let events: Vec<MenuEvent> = self
            .menu_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            if let Some(action) = self.tray.as_ref().and_then(|t| t.action(&event.id)) {
                self.handle(action);
            }
        }
        if let Some(msg) = self.login.as_mut().and_then(Login::poll) {
            self.finish_login(msg);
        }
        if let Some(Ok((id, result))) = self.emails_rx.as_ref().map(Receiver::try_recv) {
            self.emails_rx = None;
            match result {
                Ok(list) => {
                    if let Some(a) = self.model.config.accounts.iter_mut().find(|a| a.id == id) {
                        a.emails = list;
                    }
                    self.save("Loaded verified emails");
                }
                Err(e) => self.error(e),
            }
        }
        if self.last_reload.elapsed() >= Duration::from_secs(1) {
            self.last_reload = Instant::now();
            if let Some(glass) = &self.glass {
                glass.keep_egui_on_top();
            }
            match self.model.reload_if_changed() {
                Ok(true) => self.config_changed(),
                Ok(false) => {}
                Err(e) => self.error(e),
            }
        }
    }

    fn handle(&mut self, action: Action) {
        match action {
            Action::SetDefault(id) => self.set_default(id),
            Action::Open(tab) => {
                self.tab = tab;
                self.want_visible = true;
            }
            Action::Quit => std::process::exit(0),
        }
    }

    fn set_default(&mut self, id: u64) {
        self.model.config.default_account_id = Some(id);
        let login = self.model.login_of(id);
        self.save(format!("{login} is now the default account"));
    }

    fn finish_login(&mut self, msg: login::Msg) {
        match msg {
            login::Msg::Done {
                token,
                user,
                emails,
            } => {
                self.login = None;
                let added = accounts::add(
                    &mut self.model.config,
                    &self.model.store,
                    &token,
                    &user,
                    &emails,
                );
                match added {
                    Ok(Added::New) => self.save(format!(
                        "Added {}. Check its commit name and email below.",
                        user.login
                    )),
                    Ok(Added::Relogin) => self.save(format!("{} logged in again", user.login)),
                    Err(e) => self.error(e),
                }
            }
            login::Msg::Failed(e) => {
                if let Some(login) = &mut self.login {
                    login.state = login::State::Failed(e);
                }
            }
            login::Msg::Code(_) => {}
        }
    }

    /// Fetches the verified emails of an account added before gitroute
    /// stored them.
    fn load_emails(&mut self, ctx: &egui::Context, id: u64) {
        let Some(account) = self.model.config.account(id) else {
            return;
        };
        let user = User {
            id,
            login: account.login.clone(),
            avatar_url: account.avatar_url.clone(),
            name: None,
        };
        let (tx, rx) = mpsc::channel();
        let (store, locks, ctx) = (
            self.model.store.clone(),
            lock_dir(&self.model.home),
            ctx.clone(),
        );
        thread::spawn(move || {
            let result = (|| -> Result<Vec<String>> {
                let token = match store.access(id, &locks, unix_now(), github::refresh)? {
                    Access::Ready { token, .. } => token,
                    Access::NeedsRelogin(reason) => bail!("log in again first ({reason})"),
                };
                Ok(accounts::commit_emails(
                    &user,
                    &github::fetch_emails(&token)?,
                ))
            })();
            let _ = tx.send((id, result.map_err(|e| format!("{e:#}"))));
            ctx.request_repaint();
        });
        self.emails_rx = Some(rx);
    }

    fn plan(&mut self, install: bool) {
        let planned = if install {
            managed::plan_install(&self.model.global_gitconfig)
        } else {
            managed::plan_uninstall(&self.model.global_gitconfig)
        };
        match planned {
            // Already installed (maybe from the CLI): still point git at
            // this app's helper, as installing from here promises.
            Ok(change) if change.is_noop() && install => match self.use_bundled_helper() {
                Ok(()) => {
                    self.model.installed = true;
                    self.info("Already installed. git now uses the helper inside this app.");
                }
                Err(e) => self.error(e),
            },
            Ok(change) if change.is_noop() => {
                self.model.installed = false;
                self.info(format!("{} is already up to date", change.path.display()));
            }
            Ok(change) => self.pending = Some(Pending { install, change }),
            Err(e) => self.error(e),
        }
    }

    /// Records the helper shipped next to this app as the one git uses, and
    /// writes the generated files with it.
    fn use_bundled_helper(&mut self) -> Result<()> {
        let helper = self
            .model
            .install_helper
            .clone()
            .ok_or_else(|| anyhow!("git-credential-gitroute isn't next to the app"))?;
        self.model.config.helper_path = Some(helper.clone());
        self.model.save()?;
        managed::write_files(&self.model.config, &self.model.home, &helper)
    }

    fn apply_pending(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let applied = (|| -> Result<Option<PathBuf>> {
            if pending.install {
                self.use_bundled_helper()?;
            }
            pending.change.apply(&self.model.home)
        })();
        match applied {
            Ok(backup) => {
                self.model.installed = pending.install;
                let backup = backup.map_or(String::new(), |b| {
                    format!(
                        " The previous version is backed up at {}.",
                        self.model.tilde(&b)
                    )
                });
                self.info(if pending.install {
                    format!("Installed: git now asks gitroute for github.com credentials.{backup}")
                } else {
                    format!("Uninstalled: gitroute's block is gone from your gitconfig.{backup}")
                });
            }
            Err(e) => self.error(e),
        }
    }

    /// The whole window. Tests drive this directly.
    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // Fonts take effect on the next frame, so skip drawing this one.
        if !self.themed {
            self.themed = true;
            theme::install(ui.ctx());
            ui.ctx()
                .request_discard("installing gitroute's fonts and colors");
            return;
        }
        self.process();
        let ctx = ui.ctx().clone();
        let p = theme::palette(ui);
        // With glass, the sidebar is see-through: the native glass pane shows
        // through it, and the window background is painted around the pane.
        // The window's traffic lights sit inside the sidebar's top.
        let fill = if self.glass.is_some() {
            Color32::TRANSPARENT
        } else {
            p.sidebar
        };
        let sidebar = egui::Panel::left("sidebar")
            .resizable(false)
            .exact_size(glass::SIDEBAR_WIDTH)
            .show_separator_line(false)
            .frame(Frame::new().fill(fill).inner_margin(Margin {
                left: 26,
                right: 22,
                top: 52,
                bottom: 20,
            }))
            .show(ui, |ui| self.sidebar(ui));
        if self.glass.is_some() {
            let outer = sidebar.response.rect;
            let hole = egui::Rect::from_min_max(
                outer.min + vec2(glass::INSET, glass::INSET),
                pos2(
                    outer.left() + glass::SIDEBAR_WIDTH - glass::INSET,
                    outer.bottom() - glass::INSET,
                ),
            );
            paint_around(ui.painter(), outer, hole, glass::RADIUS, p.ground);
        }
        egui::CentralPanel::default()
            // Content starts below the title area, where clicks drag the
            // window instead of reaching egui.
            .frame(Frame::new().fill(p.ground).inner_margin(Margin {
                left: 36,
                right: 28,
                top: glass::TITLEBAR_HEIGHT as i8 + 8,
                bottom: 16,
            }))
            .show(ui, |ui| {
                self.banner(ui);
                ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(700.0));
                    match self.tab {
                        Tab::Accounts => self.accounts_tab(ui),
                        Tab::Rules => self.rules_tab(ui),
                        Tab::Check => self.check_tab(ui),
                        Tab::Activity => self.activity_tab(ui),
                        Tab::Setup => self.setup_tab(ui),
                    }
                    ui.add_space(24.0);
                });
            });
        self.dialogs(&ctx);
    }

    /// The wordmark, how gitroute is connected to git, and the navigation.
    /// Nothing critical sits at the bottom, where windows get cut off.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            theme::mark(ui, 22.0);
            ui.label(
                RichText::new("gitroute")
                    .text_style(theme::title())
                    .color(p.ink),
            );
        });
        ui.add_space(6.0);
        let (color, text) = match self.setup_state() {
            SetupState::Installed => (p.ok, "Connected to git"),
            SetupState::OtherHelper(_) => (p.warn, "Check setup"),
            SetupState::NotInstalled => (p.warn, "Not connected to git"),
        };
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = vec2(4.0, 2.0);
            ui.add_space(4.0);
            theme::dot(ui, color);
            let link = Button::new(RichText::new(text).small().color(p.ink_soft))
                .frame_when_inactive(false);
            if ui.add(link).on_hover_text("Open Setup").clicked() {
                self.tab = Tab::Setup;
            }
        });
        ui.add_space(22.0);
        self.nav(ui);
    }

    /// Navigation drawn as a line with stations; the current page is the
    /// filled station.
    fn nav(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.spacing_mut().item_spacing.y = 0.0;
        let count = TABS.len();
        for (i, (tab, label)) in TABS.into_iter().enumerate() {
            let selected = self.tab == tab;
            let (rect, response) =
                ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
            response
                .widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, label));
            let painter = ui.painter();
            if response.hovered() && !selected {
                painter.rect_filled(rect, CornerRadius::same(8), p.hairline.gamma_multiply(0.6));
            }
            let (x, cy) = (rect.left() + 12.0, rect.center().y);
            let rail = Stroke::new(3.0, p.ink_faint);
            // The rail stops at each ring, so stations read as hollow on
            // glass as well as on a solid sidebar.
            let r = if selected { 7.5 } else { 6.75 };
            if i > 0 {
                painter.line_segment([pos2(x, rect.top()), pos2(x, cy - r)], rail);
            }
            if i + 1 < count {
                painter.line_segment([pos2(x, cy + r), pos2(x, rect.bottom())], rail);
            }
            if selected {
                painter.circle_filled(pos2(x, cy), 7.5, p.ink);
            } else {
                painter.circle_stroke(pos2(x, cy), 5.5, Stroke::new(2.5, p.ink_faint));
            }
            let font = if selected {
                FontId::new(14.5, theme::medium())
            } else {
                FontId::new(14.5, egui::FontFamily::Proportional)
            };
            let color = if selected { p.ink } else { p.ink_soft };
            painter.text(pos2(x + 20.0, cy), Align2::LEFT_CENTER, label, font, color);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                self.tab = tab;
            }
        }
    }

    fn setup_tab(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            "Setup",
            "How gitroute is connected to git on this Mac.",
            None,
        );
        theme::card(ui).show(ui, |ui| self.install_status(ui));
    }

    fn install_status(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        ui.set_width(ui.available_width());
        let state = self.setup_state();
        let installed = !matches!(state, SetupState::NotInstalled);
        // Installed from elsewhere (e.g. the CLI) with a different helper is
        // a problem state, so it reads as one.
        let (foreign, current) = match state {
            SetupState::OtherHelper(current) => (true, current),
            _ => (false, None),
        };
        let (color, title) = match (installed, foreign) {
            (false, _) => (p.warn, "Not installed"),
            (true, true) => (p.warn, "Installed, but git uses another helper"),
            (true, false) => (p.ok, "Installed"),
        };
        ui.horizontal(|ui| {
            theme::dot(ui, color);
            ui.add(
                Label::new(
                    RichText::new(title)
                        .font(FontId::new(14.0, theme::medium()))
                        .color(p.ink),
                )
                .wrap(),
            );
        });
        if foreign {
            // The helper's folder is what tells them apart (its file name
            // is always git-credential-gitroute); the full path is on hover.
            let full = current
                .as_deref()
                .map_or("unknown path".to_string(), |p| self.model.tilde(p));
            let folder = current
                .as_deref()
                .and_then(Path::parent)
                .map_or("unknown path".to_string(), |d| {
                    format!("in {}/", shorten_path(&self.model.tilde(d), 18))
                });
            ui.add(
                Label::new(
                    RichText::new(folder)
                        .family(egui::FontFamily::Monospace)
                        .size(11.5)
                        .color(p.ink_soft),
                )
                .truncate(),
            )
            .on_hover_text(full);
            ui.add_space(4.0);
            if theme::primary_button(ui, "Use this app's helper").clicked() {
                self.plan(true);
            }
            if ui.button("Uninstall…").clicked() {
                self.plan(false);
            }
        } else if installed {
            ui.add(
                Label::new(
                    RichText::new("git asks gitroute for github.com credentials.")
                        .small()
                        .color(p.ink_soft),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            if ui.button("Uninstall…").clicked() {
                self.plan(false);
            }
        } else {
            ui.add(
                Label::new(
                    RichText::new(
                        "git doesn't use gitroute yet. Installing adds one marked block to your gitconfig.",
                    )
                    .small()
                    .color(p.ink_soft),
                )
                .wrap(),
            );
            ui.add_space(4.0);
            if theme::primary_button(ui, "Install…").clicked() {
                self.plan(true);
            }
        }
    }

    /// The latest outcome, kept until the next one or until dismissed.
    fn banner(&mut self, ui: &mut egui::Ui) {
        let Some((text, is_error)) = self.message.clone() else {
            return;
        };
        let p = theme::palette(ui);
        let (fill, accent) = if is_error {
            (p.danger.gamma_multiply(0.12), p.danger)
        } else {
            (p.card, p.ok)
        };
        let mut dismiss = false;
        Frame::new()
            .fill(fill)
            .stroke(Stroke::new(1.0, p.hairline))
            .corner_radius(CornerRadius::same(10))
            .inner_margin(Margin::symmetric(14, 8))
            .show(ui, |ui| {
                ui.set_max_width(ui.available_width().min(700.0));
                ui.horizontal(|ui| {
                    theme::dot(ui, accent);
                    let color = if is_error { p.danger } else { p.ink };
                    ui.add(Label::new(RichText::new(&text).color(color)).wrap());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        dismiss = ui.button("Dismiss").clicked();
                    });
                });
            });
        ui.add_space(18.0);
        if dismiss {
            self.message = None;
        }
    }

    fn accounts_tab(&mut self, ui: &mut egui::Ui) {
        let add = page_header(
            ui,
            "Accounts",
            "All your GitHub accounts stay signed in at once. The default covers every folder without a rule.",
            // Hidden while a sign-in runs: it would compete with the
            // sign-in card's own action and could restart it.
            self.login.is_none().then_some("Add account…"),
        );
        if add {
            self.start_login(ui.ctx());
        }
        // A login in progress (or nothing to show yet) comes first, so it's
        // never below the fold.
        if self.login.is_some() || self.model.config.accounts.is_empty() {
            self.login_panel(ui);
            ui.add_space(12.0);
        }

        let p = theme::palette(ui);
        let lines = self.model.lines(ui.visuals().dark_mode);
        let default_id = self.model.config.default_account_id;
        let loading_emails = self.emails_rx.is_some();
        // Each account's stations: the folders routed onto its line.
        let stations: Vec<(u64, Vec<(String, bool)>)> = self
            .model
            .config
            .accounts
            .iter()
            .map(|a| {
                let is_default = default_id == Some(a.id);
                let list = self
                    .model
                    .config
                    .rules
                    .iter()
                    .filter_map(|r| match r.account {
                        Some(id) if id == a.id => Some((folder_name(&r.path), false)),
                        None if is_default => Some((folder_name(&r.path), true)),
                        _ => None,
                    })
                    .collect();
                (a.id, list)
            })
            .collect();
        let (mut changed, mut make_default, mut remove, mut relogin, mut load_emails) =
            (false, None, None, false, None);

        for account in &mut self.model.config.accounts {
            let (letter, color) = lines
                .iter()
                .find(|l| l.0 == account.id)
                .map_or(('?', theme::LINE_UNKNOWN), |l| (l.1, l.2));
            let is_default = default_id == Some(account.id);
            let own = stations
                .iter()
                .find(|s| s.0 == account.id)
                .map_or(&[][..], |s| &s.1[..]);
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let mut bullet = egui::Rect::NOTHING;
                ui.horizontal(|ui| {
                    bullet = theme::bullet(ui, letter, color, 34.0, false).rect;
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(&account.login)
                                    .text_style(theme::title())
                                    .color(p.ink),
                            );
                            if is_default {
                                theme::chip(ui, "Default");
                            }
                        });
                        if account.needs_relogin {
                            ui.horizontal(|ui| {
                                theme::dot(ui, p.warn);
                                ui.label(
                                    RichText::new("Needs re-login: GitHub rejected its token")
                                        .color(p.warn),
                                );
                            });
                        } else {
                            ui.label(RichText::new("Signed in").small().color(p.ink_soft));
                        }
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Destructive, so it sits apart at the far edge.
                        if theme::danger_button(ui, "Remove account…").clicked() {
                            remove = Some(account.id);
                        }
                        ui.add_space(12.0);
                        if account.needs_relogin {
                            if theme::primary_button(ui, "Log in again").clicked() {
                                relogin = true;
                            }
                        } else if !is_default && ui.button("Make default").clicked() {
                            make_default = Some(account.id);
                        }
                    });
                });
                account_line(ui, bullet, color, own, is_default);
                ui.separator();
                ui.add_space(6.0);
                Grid::new(("account", account.id))
                    .num_columns(2)
                    .spacing(vec2(18.0, 10.0))
                    .show(ui, |ui| {
                        let name_label =
                            ui.label(RichText::new("Commit name").color(p.ink_soft)).id;
                        let name = ui
                            .add(
                                TextEdit::singleline(&mut account.commit_name)
                                    .desired_width(320.0)
                                    .margin(vec2(8.0, 5.0)),
                            )
                            .labelled_by(name_label);
                        changed |= name.lost_focus();
                        ui.end_row();

                        let email_label =
                            ui.label(RichText::new("Commit email").color(p.ink_soft)).id;
                        if account.emails.is_empty() {
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&account.commit_email).color(p.ink));
                                let button = Button::new("Load verified emails");
                                if ui.add_enabled(!loading_emails, button).clicked() {
                                    load_emails = Some(account.id);
                                }
                            });
                        } else {
                            ComboBox::from_id_salt(("email", account.id))
                                .width(320.0)
                                .icon(theme::chevrons)
                                .selected_text(account.commit_email.as_str())
                                .show_ui(ui, |ui| {
                                    for email in &account.emails {
                                        changed |= ui
                                            .selectable_value(
                                                &mut account.commit_email,
                                                email.clone(),
                                                email,
                                            )
                                            .changed();
                                    }
                                })
                                .response
                                .labelled_by(email_label);
                        }
                        ui.end_row();
                    });
            });
            ui.add_space(12.0);
        }
        if changed {
            self.save("Saved");
        }
        if let Some(id) = make_default {
            self.set_default(id);
        }
        self.confirm_remove = remove.or(self.confirm_remove);
        if let Some(id) = load_emails {
            self.load_emails(ui.ctx(), id);
        }
        if relogin {
            self.start_login(ui.ctx());
        }
    }

    fn start_login(&mut self, ctx: &egui::Context) {
        if let Some(old) = self.login.take() {
            old.cancel();
        }
        self.login = Some(Login::start(ctx.clone()));
    }

    fn login_panel(&mut self, ui: &mut egui::Ui) {
        let p = theme::palette(ui);
        let mut intent = None;
        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            match self.login.as_ref().map(|l| &l.state) {
                None => {
                    ui.label(
                        RichText::new("No accounts yet")
                            .text_style(theme::title())
                            .color(p.ink),
                    );
                    ui.add(
                        Label::new(
                            RichText::new(
                                "Use Add account… to sign in with GitHub in your browser. gitroute never sees your password.",
                            )
                            .color(p.ink_soft),
                        )
                        .wrap(),
                    );
                }
                Some(login::State::Requesting) => {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new("Asking GitHub for a code…").color(p.ink));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                intent = Some(LoginIntent::Cancel);
                            }
                        });
                    });
                }
                Some(login::State::Waiting(code)) => {
                    ui.label(
                        RichText::new("Sign in on GitHub")
                            .text_style(theme::title())
                            .color(p.ink),
                    );
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(
                            "1.  Make sure github.com is signed in as the account you want to add. A private window helps.",
                        )
                        .color(p.ink_soft),
                    );
                    ui.label(RichText::new("2.  Enter this code:").color(p.ink_soft));
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(&code.user_code)
                            .text_style(theme::code())
                            .color(p.ink),
                    );
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, "Open github.com/login/device").clicked() {
                            let _ = Command::new("open").arg(&code.verification_uri).status();
                        }
                        if ui.button("Copy code").clicked() {
                            ui.ctx().copy_text(code.user_code.clone());
                        }
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(
                            RichText::new("Waiting for you to approve on GitHub…")
                                .color(p.ink_soft),
                        );
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                intent = Some(LoginIntent::Cancel);
                            }
                        });
                    });
                }
                Some(login::State::Failed(e)) => {
                    ui.horizontal(|ui| {
                        theme::dot(ui, p.danger);
                        ui.add(
                            Label::new(RichText::new(format!("Login failed: {e}")).color(p.danger))
                                .wrap(),
                        );
                    });
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::primary_button(ui, "Try again").clicked() {
                            intent = Some(LoginIntent::Start);
                        }
                        if ui.button("Close").clicked() {
                            intent = Some(LoginIntent::Close);
                        }
                    });
                }
            }
        });
        match intent {
            Some(LoginIntent::Start) => self.start_login(ui.ctx()),
            Some(LoginIntent::Cancel) => {
                if let Some(login) = self.login.take() {
                    login.cancel();
                }
            }
            Some(LoginIntent::Close) => self.login = None,
            None => {}
        }
    }

    fn rules_tab(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            "Folder rules",
            "A repo uses the account of the closest rule above it. Folders without a rule inherit from their parent; everything else uses the default.",
            None,
        );
        let p = theme::palette(ui);
        let lines = self.model.lines(ui.visuals().dark_mode);
        let accounts: Vec<(u64, String)> = self
            .model
            .config
            .accounts
            .iter()
            .map(|a| (a.id, a.login.clone()))
            .collect();
        let default = self.model.config.default_account().map(|a| {
            (
                a.login.clone(),
                self.model.line(a.id, ui.visuals().dark_mode),
            )
        });
        let default_label = match &default {
            Some((login, _)) => format!("Use default ({login})"),
            None => "Use default (none set)".to_string(),
        };
        let labels: Vec<String> = self
            .model
            .config
            .rules
            .iter()
            // The hollow bullet already names the default account.
            .map(|r| {
                r.account
                    .map_or("Use default".to_string(), |id| self.model.login_of(id))
            })
            .collect();
        let shown: Vec<String> = self
            .model
            .config
            .rules
            .iter()
            .map(|r| self.model.tilde(&r.path))
            .collect();

        let (mut changed, mut remove) = (false, None);
        if self.model.config.rules.is_empty() {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("No rules yet").text_style(theme::title()).color(p.ink));
                let who = default.as_ref().map_or("the default account".to_string(), |(l, _)| {
                    format!("the default account, {l}")
                });
                ui.add(
                    Label::new(
                        RichText::new(format!(
                            "Every repo uses {who}. Add a folder to route the repos inside it to another account."
                        ))
                        .color(p.ink_soft),
                    )
                    .wrap(),
                );
            });
        } else {
            theme::card(ui)
                .inner_margin(Margin::symmetric(18, 6))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    for (i, rule) in self.model.config.rules.iter_mut().enumerate() {
                        if i > 0 {
                            ui.separator();
                        }
                        ui.horizontal(|ui| {
                            ui.set_min_height(46.0);
                            // A hollow bullet: this folder rides the default line.
                            let (letter, color, hollow) = match rule.account {
                                Some(id) => {
                                    let (l, c) = lines
                                        .iter()
                                        .find(|x| x.0 == id)
                                        .map_or(('?', theme::LINE_UNKNOWN), |x| (x.1, x.2));
                                    (l, c, false)
                                }
                                None => default
                                    .as_ref()
                                    .map_or(('?', theme::LINE_UNKNOWN, true), |(_, (l, c))| {
                                        (*l, *c, true)
                                    }),
                            };
                            theme::bullet(ui, letter, color, 26.0, hollow);
                            ui.add_space(4.0);
                            let folder_label = ui
                                .scope(|ui| {
                                    // Shorten in the middle: the end of a path
                                    // is what tells two rules apart.
                                    let width = (ui.available_width() - 300.0).max(140.0);
                                    ui.set_max_width(width);
                                    let full = format!("{}/", shown[i]);
                                    let chars = (width / 7.9) as usize;
                                    ui.add(
                                        Label::new(
                                            RichText::new(shorten_path(&full, chars))
                                                .monospace()
                                                .color(p.ink),
                                        )
                                        .truncate(),
                                    )
                                    .on_hover_text(full)
                                })
                                .inner
                                .id;
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if theme::danger_button(ui, "Remove").clicked() {
                                    remove = Some(rule.path.clone());
                                }
                                ui.add_space(8.0);
                                ComboBox::from_id_salt(("rule", i))
                                    .width(200.0)
                                    .icon(theme::chevrons)
                                    .selected_text(labels[i].as_str())
                                    .show_ui(ui, |ui| {
                                        for (id, login) in &accounts {
                                            changed |= ui
                                                .selectable_value(
                                                    &mut rule.account,
                                                    Some(*id),
                                                    login,
                                                )
                                                .changed();
                                        }
                                        changed |= ui
                                            .selectable_value(
                                                &mut rule.account,
                                                None,
                                                default_label.as_str(),
                                            )
                                            .changed();
                                    })
                                    .response
                                    .labelled_by(folder_label);
                            });
                        });
                    }
                });
        }
        ui.add_space(14.0);
        if theme::primary_button(ui, "Add folder…").clicked()
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Choose a folder to route")
                .set_directory(&self.model.home)
                .pick_folder()
        {
            match paths::normalize(&folder, &self.model.home, &self.model.home) {
                Ok(path) => {
                    let account = self.model.config.default_account_id;
                    self.model.config.set_rule(path.clone(), account);
                    let shown = self.model.tilde(&path);
                    self.save(format!("Added {shown}/. Pick its account in the list."));
                }
                Err(e) => self.error(e),
            }
        }
        if let Some(path) = remove {
            self.model.config.remove_rule(&path);
            let shown = self.model.tilde(&path);
            self.save(format!("Removed the rule for {shown}/"));
        } else if changed {
            self.save("Saved");
        }
    }

    fn check_tab(&mut self, ui: &mut egui::Ui) {
        page_header(
            ui,
            "Check a path",
            "Pick a folder or type a path to see which account a repo there uses, and why.",
            None,
        );
        let p = theme::palette(ui);
        ui.horizontal(|ui| {
            let path_label = ui.label(RichText::new("Path").color(p.ink_soft)).id;
            ui.add(
                TextEdit::singleline(&mut self.check_input)
                    .hint_text("~/Desktop/Code/…")
                    .font(egui::TextStyle::Monospace)
                    .desired_width((ui.available_width() - 130.0).max(200.0))
                    .margin(vec2(10.0, 7.0)),
            )
            .labelled_by(path_label);
            if ui.button("Choose folder…").clicked()
                && let Some(folder) = rfd::FileDialog::new()
                    .set_directory(&self.model.home)
                    .pick_folder()
            {
                self.check_input = self.model.tilde(&folder);
            }
        });
        ui.add_space(16.0);
        let input = self.check_input.trim().to_string();
        if input.is_empty() {
            ui.label(
                RichText::new(
                    "Everything inside a folder rule takes that rule's account; the closest rule wins.",
                )
                .color(p.ink_soft),
            );
            return;
        }
        match paths::normalize(Path::new(&input), &self.model.home, &self.model.home) {
            Ok(path) => self.route(ui, &path),
            Err(e) => {
                ui.colored_label(p.danger, format!("{e}"));
            }
        }
    }

    /// The route from a path to its account, drawn as a line: thin until the
    /// deciding rule, then the account's line down to its terminus.
    fn route(&self, ui: &mut egui::Ui, path: &Path) {
        let p = theme::palette(ui);
        let resolution = resolve::resolve(&self.model.config, path);
        let account = resolution.account;
        let (letter, color) = match account {
            Some(a) => self.model.line(a.id, ui.visuals().dark_mode),
            None => ('?', p.ink_faint),
        };

        // Stations: home (or the root), then each folder down to the path.
        let (mut at, rest) = match path.strip_prefix(&self.model.home) {
            Ok(rest) => (self.model.home.clone(), rest),
            Err(_) => (PathBuf::from("/"), path.strip_prefix("/").unwrap_or(path)),
        };
        let mut stations = vec![(
            if at == self.model.home {
                "~".to_string()
            } else {
                "/".to_string()
            },
            at.clone(),
        )];
        for part in rest.components() {
            at.push(part);
            stations.push((part.as_os_str().to_string_lossy().into_owned(), at.clone()));
        }
        let rule_at = resolution
            .rule
            .and_then(|r| stations.iter().position(|(_, s)| paths::same(s, &r.path)));
        // With no rule, the default line runs the whole way.
        let colored_from = if account.is_some() {
            rule_at.unwrap_or(0)
        } else {
            usize::MAX
        };

        theme::card(ui).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            let thin = Stroke::new(2.0, p.ink_faint);
            let thick = Stroke::new(6.0, color);
            for (i, (name, _)) in stations.iter().enumerate() {
                ui.horizontal(|ui| {
                    let (rect, _) = ui.allocate_exact_size(vec2(40.0, 30.0), Sense::hover());
                    let c = pos2(rect.left() + 16.0, rect.center().y);
                    let painter = ui.painter();
                    if i > 0 {
                        let above = if i > colored_from { thick } else { thin };
                        painter.line_segment([pos2(c.x, rect.top()), c], above);
                    }
                    let below = if i >= colored_from { thick } else { thin };
                    painter.line_segment([c, pos2(c.x, rect.bottom())], below);
                    if Some(i) == rule_at {
                        painter.circle(c, 8.0, p.card, Stroke::new(4.0, color));
                    } else if i >= colored_from {
                        painter.circle_filled(c, 2.5, p.card);
                    } else {
                        painter.circle(c, 4.0, p.card, Stroke::new(2.0, p.ink_faint));
                    }
                    let ink = if i >= colored_from { p.ink } else { p.ink_soft };
                    ui.label(RichText::new(name).monospace().color(ink));
                    if Some(i) == rule_at {
                        ui.add_space(10.0);
                        let rule = resolution.rule.map(|r| r.account.is_some());
                        theme::chip(
                            ui,
                            if rule == Some(true) {
                                "folder rule"
                            } else {
                                "rule: use default"
                            },
                        );
                    } else if i == 0 && rule_at.is_none() && account.is_some() {
                        ui.add_space(10.0);
                        theme::chip(ui, "no rule: default account");
                    }
                });
            }
            // The terminus: the account this path ends up with.
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(40.0, 52.0), Sense::hover());
                let c = pos2(rect.left() + 16.0, rect.center().y + 6.0);
                let painter = ui.painter();
                let line = if account.is_some() { thick } else { thin };
                painter.line_segment([pos2(c.x, rect.top()), c], line);
                theme::paint_bullet(painter, c, 30.0, letter, color, account.is_none(), p.card);
                ui.add_space(4.0);
                ui.vertical(|ui| {
                    ui.add_space(12.0);
                    match account {
                        Some(a) => {
                            ui.label(
                                RichText::new(&a.login)
                                    .text_style(theme::title())
                                    .color(p.ink),
                            );
                            ui.add_space(2.0);
                            ui.label(
                                RichText::new(format!(
                                    "commits as {} <{}>",
                                    a.commit_name, a.commit_email
                                ))
                                .small()
                                .color(p.ink_soft),
                            );
                        }
                        None => {
                            ui.label(
                                RichText::new("No account: no rule matches and no default is set")
                                    .color(p.warn),
                            );
                        }
                    }
                });
            });
            if account.is_some_and(|a| a.needs_relogin) {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    theme::dot(ui, p.warn);
                    ui.label(
                        RichText::new("That account needs to log in again (Accounts).")
                            .color(p.warn),
                    );
                });
            }
            ui.add_space(14.0);
            ui.spacing_mut().item_spacing.y = 8.0;
            ui.separator();
            ui.add_space(6.0);
            let home = format!("{}/", self.model.home.display());
            let explained = resolution.to_string().replace(&home, "~/");
            // Only the path part shortens; the account it lands on stays.
            let (route, landing) = explained
                .rsplit_once(" → ")
                .map_or((explained.as_str(), ""), |(a, b)| (a, b));
            ui.horizontal(|ui| {
                let landing = format!("→ {landing}");
                let reserve = landing.chars().count() as f32 * 8.0 + 16.0;
                ui.scope(|ui| {
                    ui.set_max_width((ui.available_width() - reserve).max(80.0));
                    ui.add(
                        Label::new(RichText::new(route).monospace().color(p.ink_soft)).truncate(),
                    )
                    .on_hover_text(&explained);
                });
                ui.label(RichText::new(landing).monospace().color(p.ink));
            });
        });
    }

    fn activity_tab(&mut self, ui: &mut egui::Ui) {
        if self
            .activity_read
            .is_none_or(|t| t.elapsed() > Duration::from_secs(2))
        {
            self.activity_read = Some(Instant::now());
            match helper_log::read_recent(&self.model.home, 200) {
                Ok(entries) => self.activity = entries,
                Err(e) => self.error(e),
            }
            ui.ctx().request_repaint_after(Duration::from_secs(2));
        }
        page_header(
            ui,
            "Activity",
            "Each time git asked gitroute for credentials, newest first. Tokens are never logged.",
            None,
        );
        let p = theme::palette(ui);
        if self.activity.is_empty() {
            theme::card(ui).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Nothing yet").text_style(theme::title()).color(p.ink));
                ui.add(
                    Label::new(
                        RichText::new(
                            "Run any git command over HTTPS in a routed folder, such as git fetch, and it shows up here.",
                        )
                        .color(p.ink_soft),
                    )
                    .wrap(),
                );
            });
            return;
        }
        let now = unix_now();
        let lines = self.model.lines(ui.visuals().dark_mode);
        theme::card(ui)
            .inner_margin(Margin::symmetric(18, 12))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                Grid::new("activity")
                    .num_columns(3)
                    .spacing(vec2(14.0, 14.0))
                    .show(ui, |ui| {
                        for entry in &self.activity {
                            ui.label(
                                RichText::new(ago(now, entry.time))
                                    .small()
                                    .color(p.ink_soft),
                            );
                            let line = entry.account.as_deref().and_then(|login| {
                                let account = self.model.config.account_by_login(login)?;
                                lines.iter().find(|l| l.0 == account.id)
                            });
                            match line {
                                Some(&(_, letter, color)) => {
                                    theme::bullet(ui, letter, color, 20.0, false);
                                }
                                None => {
                                    ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
                                }
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.add(
                                    Label::new(
                                        RichText::new(self.model.tilde(Path::new(&entry.path)))
                                            .monospace()
                                            .color(p.ink),
                                    )
                                    .wrap(),
                                );
                                let ok = entry.outcome.starts_with("token sent");
                                ui.label(
                                    RichText::new(format!("{}: {}", entry.action, entry.outcome))
                                        .small()
                                        .color(if ok { p.ink_soft } else { p.warn }),
                                );
                            });
                            ui.end_row();
                        }
                    });
            });
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(pending) = &self.pending {
            let mut choice = None;
            let frame = Frame::popup(&ctx.global_style()).inner_margin(Margin::same(24));
            let modal = egui::Modal::new(egui::Id::unique("gitconfig-change")).frame(frame).show(ctx, |ui| {
                let p = theme::palette(ui);
                ui.set_width(620.0);
                let (title, intro, apply) = if pending.install {
                    (
                        "Install gitroute into git?",
                        "This adds one marked block at the end of",
                        "Install",
                    )
                } else {
                    (
                        "Remove gitroute from git?",
                        "This removes gitroute's marked block from",
                        "Uninstall",
                    )
                };
                ui.label(RichText::new(title).text_style(theme::title()).color(p.ink));
                ui.add_space(4.0);
                ui.add(
                    Label::new(
                        RichText::new(format!(
                            "{intro} {}. Nothing else in the file changes, and the current version is backed up first.",
                            self.model.tilde(&pending.change.path)
                        ))
                        .color(p.ink_soft),
                    )
                    .wrap(),
                );
                ui.add_space(12.0);
                Frame::new()
                    .fill(p.field)
                    .stroke(Stroke::new(1.0, p.hairline))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::same(12))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        ScrollArea::vertical()
                            .max_height(320.0)
                            .show(ui, |ui| diff_view(ui, &pending.change.diff()));
                    });
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if pending.install {
                            if theme::primary_button(ui, apply).clicked() {
                                choice = Some(true);
                            }
                            if ui.button("Cancel").clicked() {
                                choice = Some(false);
                            }
                        } else {
                            // Removing is the destructive choice, so Cancel
                            // is the default.
                            if theme::primary_button(ui, "Cancel").clicked() {
                                choice = Some(false);
                            }
                            if destructive_button(ui, apply).clicked() {
                                choice = Some(true);
                            }
                        }
                    });
                });
            });
            if modal.should_close() {
                choice = choice.or(Some(false));
            }
            match choice {
                Some(true) => self.apply_pending(),
                Some(false) => self.pending = None,
                None => {}
            }
        }

        if let Some(id) = self.confirm_remove {
            let login = self.model.login_of(id);
            let mut choice = None;
            let frame = Frame::popup(&ctx.global_style()).inner_margin(Margin::same(24));
            let modal = egui::Modal::new(egui::Id::unique("remove-account")).frame(frame).show(ctx, |ui| {
                let p = theme::palette(ui);
                ui.set_width(440.0);
                ui.label(
                    RichText::new(format!("Remove {login}?"))
                        .text_style(theme::title())
                        .color(p.ink),
                );
                ui.add_space(4.0);
                ui.add(
                    Label::new(
                        RichText::new(
                            "Its token is deleted from the Keychain. Folder rules that point at it are kept, and work again if you add the account back.",
                        )
                        .color(p.ink_soft),
                    )
                    .wrap(),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if theme::primary_button(ui, "Cancel").clicked() {
                            choice = Some(false);
                        }
                        if destructive_button(ui, "Remove").clicked() {
                            choice = Some(true);
                        }
                    });
                });
            });
            if modal.should_close() {
                choice = choice.or(Some(false));
            }
            match choice {
                Some(true) => {
                    self.confirm_remove = None;
                    match accounts::remove(&mut self.model.config, &self.model.store, id) {
                        Ok(_) => self.save(format!("Removed {login} and its token")),
                        Err(e) => self.error(e),
                    }
                }
                Some(false) => self.confirm_remove = None,
                None => {}
            }
        }
    }
}

impl eframe::App for App {
    /// Transparent, so the native glass shows through the sidebar; every
    /// other region is painted by egui.
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0; 4]
    }

    /// Runs before every frame, and while the window is hidden.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process();
        if ctx.input(|i| i.viewport().close_requested()) {
            // Closing the window keeps gitroute in the menu bar.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.want_visible = false;
        } else if self.want_visible {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            // A single repaint doesn't always bring a hidden window back
            // (docs/NOTES.md, experiment 4), so keep asking until it shows.
            if ctx.input(|i| i.viewport().visible()) == Some(true) {
                self.want_visible = false;
            } else {
                ctx.request_repaint_after(Duration::from_millis(50));
            }
        }
        // Keep noticing config changes made by the CLI or the helper.
        ctx.request_repaint_after(Duration::from_secs(1));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}

/// Paints `color` over `outer` except a rounded `hole`: the window
/// background around the floating glass sidebar.
fn paint_around(
    painter: &egui::Painter,
    outer: egui::Rect,
    hole: egui::Rect,
    radius: f32,
    color: Color32,
) {
    use std::f32::consts::{FRAC_PI_2, PI};
    for band in [
        egui::Rect::from_min_max(outer.min, pos2(outer.right(), hole.top())),
        egui::Rect::from_min_max(pos2(outer.left(), hole.bottom()), outer.max),
        egui::Rect::from_min_max(
            pos2(outer.left(), hole.top()),
            pos2(hole.left(), hole.bottom()),
        ),
        egui::Rect::from_min_max(
            pos2(hole.right(), hole.top()),
            pos2(outer.right(), hole.bottom()),
        ),
    ] {
        painter.rect_filled(band, 0.0, color);
    }
    // Each corner: the square minus a quarter circle, as a triangle fan.
    let r = radius;
    let mut mesh = egui::Mesh::default();
    for (corner, center, start) in [
        (hole.left_top(), hole.left_top() + vec2(r, r), PI),
        (hole.right_top(), hole.right_top() + vec2(-r, r), 1.5 * PI),
        (hole.right_bottom(), hole.right_bottom() + vec2(-r, -r), 0.0),
        (
            hole.left_bottom(),
            hole.left_bottom() + vec2(r, -r),
            FRAC_PI_2,
        ),
    ] {
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, color);
        const STEPS: u32 = 16;
        for i in 0..=STEPS {
            let angle = start + i as f32 / STEPS as f32 * FRAC_PI_2;
            mesh.colored_vertex(center + vec2(angle.cos(), angle.sin()) * r, color);
        }
        for i in 0..STEPS {
            mesh.add_triangle(base, base + 1 + i, base + 2 + i);
        }
    }
    painter.add(mesh);
    // An anti-aliased edge over the fans' hard one.
    painter.rect_stroke(hole, r, Stroke::new(1.0, color), egui::StrokeKind::Outside);
}

/// The page title, its one-line intro, and optionally the page's main action
/// at the trailing edge. Returns whether the action was clicked.
fn page_header(ui: &mut egui::Ui, title: &str, intro: &str, action: Option<&str>) -> bool {
    let p = theme::palette(ui);
    let mut clicked = false;
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).heading().color(p.ink));
        if let Some(action) = action {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                clicked = theme::primary_button(ui, action).clicked();
            });
        }
    });
    ui.add_space(2.0);
    ui.add(Label::new(RichText::new(intro).color(p.ink_soft)).wrap());
    ui.add_space(22.0);
    clicked
}

/// A destructive choice inside a confirmation: neutral fill, red label.
fn destructive_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let p = theme::palette(ui);
    ui.add(Button::new(RichText::new(text).color(p.danger)))
}

/// The account drawn as its line: a rail out of its bullet, a station per
/// folder routed onto it (hollow when it rides the default), and on the
/// default's line a hollow terminus for everything else.
fn account_line(
    ui: &mut egui::Ui,
    bullet: egui::Rect,
    color: Color32,
    stations: &[(String, bool)],
    is_default: bool,
) {
    let p = theme::palette(ui);
    let mut items: Vec<(String, bool)> = stations.to_vec();
    if is_default {
        items.push(("everything else".to_string(), true));
    }
    if items.len() > 5 {
        let more = items.len() - 4;
        items.truncate(4);
        items.push((format!("+{more} more"), true));
    }
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 50.0), Sense::hover());
    let painter = ui.painter();
    let (x0, y) = (bullet.center().x, rect.top() + 14.0);
    let rail = Stroke::new(4.0, color);
    painter.line_segment([pos2(x0, bullet.bottom()), pos2(x0, y)], rail);
    if items.is_empty() {
        painter.line_segment([pos2(x0, y), pos2(x0 + 36.0, y)], rail);
        painter.text(
            pos2(x0 + 48.0, y),
            Align2::LEFT_CENTER,
            "No folders routed here yet",
            FontId::new(12.0, egui::FontFamily::Proportional),
            p.ink_soft,
        );
        return;
    }
    let slot = ((rect.right() - x0 - 48.0) / items.len() as f32).min(150.0);
    let first = x0 + 48.0;
    let last = first + slot * (items.len() - 1) as f32;
    painter.line_segment([pos2(x0, y), pos2(last, y)], rail);
    let chars = ((slot - 8.0) / 6.8).max(4.0) as usize;
    for (i, (label, hollow)) in items.iter().enumerate() {
        let c = pos2(first + slot * i as f32, y);
        if *hollow {
            painter.circle(c, 5.5, p.card, Stroke::new(3.0, color));
        } else {
            painter.circle_filled(c, 6.5, color);
            painter.circle_filled(c, 2.5, p.card);
        }
        painter.text(
            c + vec2(0.0, 12.0),
            Align2::CENTER_TOP,
            middle_truncate(label, chars),
            FontId::new(11.5, egui::FontFamily::Monospace),
            p.ink_soft,
        );
    }
}

/// `Acme/` for `/x/Acme`.
fn folder_name(path: &Path) -> String {
    path.file_name()
        .map_or("/".to_string(), |n| format!("{}/", n.to_string_lossy()))
}

/// Shortens a path to about `max` characters on `/` boundaries:
/// `/Users/mona/…/bin/git-credential-gitroute`. Keeps whole trailing
/// components first (they tell paths apart), then leading ones; a single
/// component too long to fit is cut in its middle.
fn shorten_path(path: &str, max: usize) -> String {
    if path.chars().count() <= max {
        return path.to_string();
    }
    let parts: Vec<&str> = path.split('/').collect();
    let len = |v: &[&str]| v.iter().map(|p| p.chars().count() + 1).sum::<usize>();
    let mut tail: Vec<&str> = Vec::new();
    for part in parts.iter().rev() {
        if !tail.is_empty() && len(&tail) + part.chars().count() + 3 > max {
            break;
        }
        tail.insert(0, part);
    }
    let rest = &parts[..parts.len() - tail.len()];
    // Always keep the first component (`~` or the root), so a shortened
    // home path never reads as an absolute one.
    let mut head: Vec<&str> = rest.first().copied().into_iter().collect();
    for part in rest.iter().skip(1) {
        if len(&head) + part.chars().count() + 2 + len(&tail) > max {
            break;
        }
        head.push(part);
    }
    let short = format!("{}/…/{}", head.join("/"), tail.join("/"));
    if short.chars().count() > max + 2 {
        middle_truncate(path, max)
    } else {
        short
    }
}

/// Shortens `text` to `max` characters by cutting the middle, keeping the
/// start and the more telling end.
fn middle_truncate(text: &str, max: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max || max < 5 {
        return text.to_string();
    }
    // Favor the end: it's what tells two paths apart.
    let head = (max - 1) * 3 / 7;
    let tail = max - 1 - head;
    let start: String = chars[..head].iter().collect();
    let end: String = chars[chars.len() - tail..].iter().collect();
    format!("{start}…{end}")
}

/// The diff body; the dialog's sentence already names the file, so the
/// `---`/`+++` header lines are left out.
fn diff_view(ui: &mut egui::Ui, diff: &str) {
    let p = theme::palette(ui);
    ui.spacing_mut().item_spacing.y = 3.0;
    for line in diff
        .lines()
        .filter(|l| !l.starts_with("---") && !l.starts_with("+++"))
    {
        let color = match line.chars().next() {
            Some('+') if !line.starts_with("+++") => p.ok,
            Some('-') if !line.starts_with("---") => p.danger,
            _ => p.ink_soft,
        };
        ui.label(RichText::new(line).monospace().color(color));
    }
}

/// "12 s ago", "5 min ago", "3 h ago", "2 d ago".
fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    match secs {
        0..60 => format!("{secs} s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

/// Where the app keeps its helper: next to its own binary.
pub fn bundled_helper() -> Option<PathBuf> {
    managed::helper_next_to_current_exe()
        .context("looking for git-credential-gitroute")
        .ok()
}

#[cfg(test)]
mod tests;
