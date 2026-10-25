//! gitroute menu bar app + settings window. Thin layer over gitroute-core.

mod app;
mod login;
mod theme;
mod tray;

use std::sync::mpsc;

use eframe::egui;
use gitroute_core::managed;
use gitroute_core::token_store::TokenStore;
use tray_icon::menu::MenuEvent;

fn main() -> eframe::Result {
    let Some(home) = std::env::home_dir() else {
        eprintln!("gitroute: can't determine your home directory");
        std::process::exit(1);
    };
    let model = match app::Model::load(
        home.clone(),
        managed::global_gitconfig(&home),
        TokenStore::from_env(),
        app::bundled_helper(),
    ) {
        Ok(model) => model,
        Err(e) => {
            eprintln!("gitroute: {e:#}");
            std::process::exit(1);
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gitroute")
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([760.0, 480.0]),
        // Menu bar only: no Dock icon, even when run outside the .app bundle.
        event_loop_builder: Some(Box::new(|builder| {
            use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
            builder.with_activation_policy(ActivationPolicy::Accessory);
        })),
        ..Default::default()
    };
    eframe::run_native(
        "gitroute",
        options,
        Box::new(move |cc| {
            // The tray must be created here: main thread, event loop running.
            let tray = tray::Tray::new(&model.config)?;
            // Once a handler is set, MenuEvent::receiver() gets nothing, so
            // forward events to our own channel and wake App::logic, which
            // runs even while the window is hidden.
            let (tx, rx) = mpsc::channel();
            let ctx = cc.egui_ctx.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                let _ = tx.send(event);
                ctx.request_repaint();
            }));
            Ok(Box::new(app::App::new(model, Some(tray), Some(rx))))
        }),
    )
}
