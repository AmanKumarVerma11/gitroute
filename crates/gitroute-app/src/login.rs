//! The device flow on a background thread, so the window stays responsive
//! while GitHub waits for the user. Cancelling stops the polling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;
use std::time::Duration;

use anyhow::{Result, bail};
use eframe::egui;
use gitroute_core::github::{self, DeviceCode, Email, User};
use gitroute_core::token_store::Token;

pub enum Msg {
    Code(DeviceCode),
    Done {
        token: Token,
        user: User,
        emails: Vec<Email>,
    },
    Failed(String),
}

pub enum State {
    Requesting,
    Waiting(DeviceCode),
    Failed(String),
}

pub struct Login {
    pub state: State,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
}

impl Login {
    pub fn start(ctx: egui::Context) -> Login {
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        thread::spawn(move || {
            let send = |msg| {
                let _ = tx.send(msg);
                ctx.request_repaint();
            };
            match run(&flag, |code| send(Msg::Code(code))) {
                Ok((token, user, emails)) => send(Msg::Done {
                    token,
                    user,
                    emails,
                }),
                Err(_) if flag.load(Ordering::SeqCst) => {}
                Err(e) => send(Msg::Failed(format!("{e:#}"))),
            }
        });
        Login::from_channel(rx, cancel)
    }

    pub fn from_channel(rx: Receiver<Msg>, cancel: Arc<AtomicBool>) -> Login {
        Login {
            state: State::Requesting,
            rx,
            cancel,
        }
    }

    /// The next message that ends the flow (`Done` or `Failed`), if any.
    /// A `Code` updates the state.
    pub fn poll(&mut self) -> Option<Msg> {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Code(code)) => self.state = State::Waiting(code),
                Ok(msg) => return Some(msg),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    return Some(Msg::Failed("the login stopped unexpectedly".into()));
                }
            }
        }
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }
}

fn run(cancel: &AtomicBool, on_code: impl FnOnce(DeviceCode)) -> Result<(Token, User, Vec<Email>)> {
    let code = github::request_device_code()?;
    on_code(code.clone());
    let token = github::wait_for_token(
        &code,
        || {
            if cancel.load(Ordering::SeqCst) {
                bail!("cancelled");
            }
            github::poll_once(&code.device_code)
        },
        |wait| sleep_unless_cancelled(wait, cancel),
    )?;
    let user = github::fetch_user(&token.access_token)?;
    let emails = github::fetch_emails(&token.access_token)?;
    Ok((token, user, emails))
}

fn sleep_unless_cancelled(wait: Duration, cancel: &AtomicBool) {
    let step = Duration::from_millis(100);
    let mut left = wait;
    while !left.is_zero() && !cancel.load(Ordering::SeqCst) {
        let nap = left.min(step);
        thread::sleep(nap);
        left -= nap;
    }
}
