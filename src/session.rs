//! Une session = un joueur connecté : lit le clavier, envoie les différences d'écran.

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use ratatui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::Rect,
};
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, server::Msg};
use tokio::time::{MissedTickBehavior, interval, timeout};

use crate::{
    game::{Game, MAX_MESSAGE, MAX_NAME, chat_char, clean_name},
    input::{Key, Parser},
    ui::{self, View},
};

const FRAME: Duration = Duration::from_millis(100);
const IDLE: Duration = Duration::from_secs(600);
/// Un client qui ne lit plus est coupé au lieu d'accumuler de la mémoire.
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[?2004h\x1b[2J";
const EXIT: &[u8] = b"\x1b[0m\x1b[?2004l\x1b[?25h\x1b[?1049l";

pub type Shared = Arc<Mutex<Game>>;

fn lock(g: &Shared) -> std::sync::MutexGuard<'_, Game> {
    g.lock().unwrap_or_else(|e| e.into_inner())
}

/// Retire le joueur même si la session se termine en erreur.
struct Seat {
    game: Shared,
    id: u64,
}

impl Drop for Seat {
    fn drop(&mut self) {
        lock(&self.game).leave(self.id, Instant::now());
    }
}

fn area(cols: u32, rows: u32) -> Rect {
    // Taille fournie par le client : bornée pour limiter la mémoire.
    Rect::new(0, 0, cols.clamp(1, 300) as u16, rows.clamp(1, 100) as u16)
}

struct Screen {
    prev: Buffer,
    size: Rect,
    redraw: bool,
}

impl Screen {
    fn frame(&mut self, view: View) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        if self.redraw {
            self.prev = Buffer::empty(self.size);
            self.redraw = false;
            out.extend_from_slice(b"\x1b[0m\x1b[2J");
        }
        let mut next = Buffer::empty(self.size);
        ui::draw(&mut next, view, Instant::now());
        let diff: Vec<_> = self.prev.diff(&next);
        if !diff.is_empty() {
            CrosstermBackend::new(&mut out).draw(diff.into_iter())?;
        }
        out.flush()?;
        self.prev = next;
        Ok(out)
    }
}

async fn send(tx: &ChannelWriteHalf<Msg>, data: Vec<u8>) -> Result<()> {
    if !data.is_empty() {
        timeout(WRITE_TIMEOUT, tx.data_bytes(data)).await??;
    }
    Ok(())
}

pub async fn run(
    game: Shared,
    id: u64,
    user: String,
    mut rx: ChannelReadHalf,
    tx: ChannelWriteHalf<Msg>,
) {
    let mut tty = false;
    let result = play(game, id, user, &mut rx, &tx, &mut tty).await;
    if let Err(e) = &result {
        eprintln!("session {id}: {e}");
    }
    let _ = timeout(WRITE_TIMEOUT, async {
        if tty {
            let _ = tx.data_bytes(EXIT.to_vec()).await;
        }
        let _ = tx.exit_status(0).await;
        let _ = tx.eof().await;
        let _ = tx.close().await;
    })
    .await;
}

async fn play(
    game: Shared,
    id: u64,
    user: String,
    rx: &mut ChannelReadHalf,
    tx: &ChannelWriteHalf<Msg>,
    tty: &mut bool,
) -> Result<()> {
    // Attend pty + shell avant de dessiner.
    let mut size = None;
    timeout(Duration::from_secs(15), async {
        loop {
            match rx.wait().await {
                Some(ChannelMsg::RequestPty { col_width, row_height, .. }) => {
                    size = Some(area(col_width, row_height));
                }
                Some(ChannelMsg::RequestShell { .. }) => return Ok(()),
                Some(_) => {}
                None => bail!("canal fermé"),
            }
        }
    })
    .await??;
    let Some(size) = size else {
        send(tx, b"Un terminal interactif est requis (ssh -t).\r\n".to_vec()).await?;
        return Ok(());
    };

    *tty = true;
    send(tx, ENTER.to_vec()).await?;
    let mut screen = Screen { prev: Buffer::empty(size), size, redraw: true };
    let mut parser = Parser::default();
    let mut name = clean_name(&user);
    let mut seat: Option<Seat> = None;
    let mut draft = String::new();
    let mut last_said: Option<Instant> = None;
    let mut last_input = Instant::now();
    let mut ticker = interval(FRAME);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            msg = rx.wait() => match msg {
                Some(ChannelMsg::Data { data }) => {
                    last_input = Instant::now();
                    for key in parser.feed(&data) {
                        match (key, &seat) {
                            (Key::Quit, _) => return Ok(()),
                            (Key::Enter, None) => {
                                lock(&game).join(id, &name, Instant::now());
                                seat = Some(Seat { game: game.clone(), id });
                            }
                            (Key::Backspace, None) => { name.pop(); }
                            (Key::Char(c), None) => {
                                if name.len() < MAX_NAME {
                                    name.push_str(&clean_name(&c.to_string()));
                                }
                            }
                            (Key::Enter, Some(_)) if draft.trim().is_empty() => {
                                draft.clear();
                                // Un double Entrée après un message ne change pas « prêt ».
                                if last_said.is_none_or(|t| t.elapsed() > Duration::from_secs(1)) {
                                    lock(&game).press_enter(id, Instant::now());
                                }
                            }
                            (Key::Enter, Some(_)) => {
                                let now = Instant::now();
                                if lock(&game).say(id, &draft, now) {
                                    draft.clear();
                                    last_said = Some(now);
                                }
                            }
                            (Key::Char(c), Some(_)) => {
                                let now = Instant::now();
                                let mut g = lock(&game);
                                if !g.chat_open(now) {
                                    g.key(id, c, now);
                                } else if chat_char(c) && draft.chars().count() < MAX_MESSAGE {
                                    draft.push(c);
                                }
                            }
                            (Key::Backspace, Some(_)) => {
                                if lock(&game).chat_open(Instant::now()) {
                                    draft.pop();
                                }
                            }
                        }
                    }
                }
                Some(ChannelMsg::WindowChange { col_width, row_height, .. }) => {
                    screen.size = area(col_width, row_height);
                    screen.redraw = true;
                }
                Some(ChannelMsg::Eof | ChannelMsg::Close) | None => return Ok(()),
                Some(_) => continue,
            },
            _ = ticker.tick() => {
                if last_input.elapsed() > IDLE {
                    return Ok(());
                }
            }
        }

        let out = if seat.is_some() {
            let snapshot = lock(&game).clone();
            screen.frame(View::Game(&snapshot, id, &draft))?
        } else {
            screen.frame(View::Naming(&name))?
        };
        send(tx, out).await?;
    }
}
