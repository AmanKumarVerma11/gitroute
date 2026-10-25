//! The menu bar icon and its menu. The menu is rebuilt from the config
//! whenever it changes.

use std::collections::HashMap;

use anyhow::Result;
use gitroute_core::config::Config;
use tray_icon::menu::{CheckMenuItem, Menu, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

use crate::app::Tab;

#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SetDefault(u64),
    Open(Tab),
    Quit,
}

pub struct Tray {
    /// Dropping this removes the icon from the menu bar.
    icon: TrayIcon,
    actions: HashMap<MenuId, Action>,
}

impl Tray {
    /// Must run on the main thread after the event loop has started
    /// (docs/NOTES.md, experiment 4).
    pub fn new(config: &Config) -> Result<Tray> {
        let (menu, actions) = build_menu(config)?;
        let icon = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("gitroute")
            .with_icon_templated(icon())
            .build()?;
        Ok(Tray { icon, actions })
    }

    pub fn update(&mut self, config: &Config) -> Result<()> {
        let (menu, actions) = build_menu(config)?;
        self.icon.set_menu(Some(Box::new(menu)));
        self.actions = actions;
        Ok(())
    }

    pub fn action(&self, id: &MenuId) -> Option<Action> {
        self.actions.get(id).cloned()
    }
}

/// Default account (checked; click another to switch), then each account's
/// status, then the window shortcuts.
fn build_menu(config: &Config) -> Result<(Menu, HashMap<MenuId, Action>)> {
    let menu = Menu::new();
    let mut actions = HashMap::new();

    menu.append(&MenuItem::new("Default account", false, None))?;
    if config.accounts.is_empty() {
        menu.append(&MenuItem::new("No accounts yet", false, None))?;
    }
    for account in &config.accounts {
        let label = if account.needs_relogin {
            format!("{} (needs re-login)", account.login)
        } else {
            account.login.clone()
        };
        let checked = config.default_account_id == Some(account.id);
        let item = CheckMenuItem::new(label, true, checked, None);
        actions.insert(item.id().clone(), Action::SetDefault(account.id));
        menu.append(&item)?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    for (label, action) in [
        ("Open gitroute…", Action::Open(Tab::Accounts)),
        ("Check a path…", Action::Open(Tab::Check)),
    ] {
        let item = MenuItem::new(label, true, None);
        actions.insert(item.id().clone(), action);
        menu.append(&item)?;
    }
    menu.append(&PredefinedMenuItem::separator())?;
    let quit = MenuItem::new("Quit gitroute", true, None);
    actions.insert(quit.id().clone(), Action::Quit);
    menu.append(&quit)?;
    Ok((menu, actions))
}

/// A small branch glyph, drawn in black with alpha so macOS can tint it as a
/// template image for light and dark menu bars.
fn icon() -> Icon {
    const SIZE: u32 = 36;
    let segments = [((12.0, 7.0), (12.0, 29.0)), ((25.0, 11.0), (14.0, 24.0))];
    let dots = [(12.0, 7.0), (12.0, 29.0), (25.0, 11.0)];
    let mut rgba = Vec::with_capacity((SIZE * SIZE * 4) as usize);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let line = segments
                .iter()
                .map(|&(a, b)| coverage(segment_distance(p, a, b), 1.5))
                .fold(0.0, f32::max);
            let dot = dots
                .iter()
                .map(|&c| coverage(distance(p, c), 4.0))
                .fold(0.0, f32::max);
            rgba.extend_from_slice(&[0, 0, 0, (line.max(dot) * 255.0) as u8]);
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).expect("icon size matches its pixels")
}

/// Anti-aliased coverage of a shape of `radius` at `distance`.
fn coverage(distance: f32, radius: f32) -> f32 {
    (radius + 0.5 - distance).clamp(0.0, 1.0)
}

fn distance(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let t = (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
    distance(p, (a.0 + t * dx, a.1 + t * dy))
}
