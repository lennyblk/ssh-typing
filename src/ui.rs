//! Rendu. Palette 256 couleurs : COLORTERM ne traverse pas SSH.

use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::game::{Game, MAX_RACERS, Phase, Player};

const ACCENT: Color = Color::Indexed(215);
const DIM: Color = Color::Indexed(245);
const HAIRLINE: Color = Color::Indexed(239);
const TRACK: Color = Color::Indexed(237);
const KEY_BG: Color = Color::Indexed(238);
const TEXT: Color = Color::Indexed(252);
const GOOD: Color = Color::Indexed(114);
const BAD: Color = Color::Indexed(203);

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 18;

const LOGO: [&str; 3] = [
    "█▀▀ █▀▀ █ █   ▀█▀ █ █ █▀█ ▀█▀ █▄ █ █▀▀",
    "▀▀█ ▀▀█ █▀█    █  ▀█▀ █▀▀  █  █ ▀█ █ █",
    "▀▀▀ ▀▀▀ ▀ ▀    ▀   ▀  ▀   ▀▀▀ ▀  ▀ ▀▀▀",
];

pub enum View<'a> {
    Naming(&'a str),
    Game(&'a Game, u64),
}

fn fg(c: Color) -> Style {
    Style::new().fg(c)
}

fn bold(c: Color) -> Style {
    fg(c).add_modifier(Modifier::BOLD)
}

fn put(buf: &mut Buffer, area: Rect, y: u16, line: Line) {
    if y < area.bottom() {
        Paragraph::new(line).render(Rect { y, height: 1, ..area }, buf);
    }
}

fn center(buf: &mut Buffer, area: Rect, y: u16, line: Line) {
    put(buf, area, y, line.centered());
}

fn secs_left(until: Instant, now: Instant) -> u64 {
    until.saturating_duration_since(now).as_millis().div_ceil(1000) as u64
}

pub fn draw(buf: &mut Buffer, view: View, now: Instant) {
    let full = buf.area;
    if full.width < MIN_W || full.height < MIN_H {
        let msg = format!("terminal trop petit ({MIN_W}x{MIN_H} min)");
        return center(buf, full, full.height / 2, Line::styled(msg, fg(BAD)));
    }
    let w = (full.width - 4).min(92);
    let area = Rect { x: (full.width - w) / 2, y: 1, width: w, height: full.height - 2 };

    match view {
        View::Naming(name) => naming(buf, area, name),
        View::Game(g, me) => match g.phase {
            Phase::Lobby { start_at } => lobby(buf, area, g, me, start_at, now),
            Phase::Racing { start, ends } => race(buf, area, g, me, start, ends, now),
            Phase::Results { until } => results(buf, area, g, me, until, now),
        },
    }
}

fn footer(buf: &mut Buffer, area: Rect, keys: &[(&str, &str)]) {
    let mut spans = Vec::new();
    for (k, label) in keys.iter().chain(&[("Ctrl+C", "quitter")]) {
        spans.push(Span::styled(format!(" {k} "), bold(TEXT).bg(KEY_BG)));
        spans.push(Span::styled(format!(" {label}   "), fg(DIM)));
    }
    center(buf, area, area.bottom() - 1, Line::from(spans));
}

fn logo(buf: &mut Buffer, area: Rect) -> u16 {
    for (i, l) in LOGO.iter().enumerate() {
        center(buf, area, area.y + i as u16, Line::styled(*l, fg(ACCENT)));
    }
    area.y + LOGO.len() as u16 + 1
}

fn header(buf: &mut Buffer, area: Rect, title: &str, right: String, color: Color) -> u16 {
    put(buf, area, area.y, Line::styled(format!("▍{title}"), bold(ACCENT)));
    put(buf, area, area.y, Line::styled(right, bold(color)).right_aligned());
    rule(buf, area, area.y + 1);
    area.y + 3
}

fn rule(buf: &mut Buffer, area: Rect, y: u16) {
    put(buf, area, y, Line::styled("─".repeat(area.width as usize), fg(HAIRLINE)));
}

fn naming(buf: &mut Buffer, area: Rect, name: &str) {
    let y = logo(buf, area);
    center(buf, area, y, Line::styled("course de frappe sous la cendre", fg(DIM)));
    center(
        buf,
        area,
        y + 4,
        Line::from(vec![
            Span::styled("Ton nom  ", fg(DIM)),
            Span::styled(name.to_string(), bold(TEXT)),
            Span::styled("█", fg(ACCENT)),
        ]),
    );
    footer(buf, area, &[("Entrée", "rejoindre")]);
}

/// Fenêtre de `max` lignes contenant toujours le joueur courant.
fn window<'a>(list: &[&'a Player], me: u64, max: usize) -> Vec<(usize, &'a Player)> {
    let mine = list.iter().position(|p| p.id == me).unwrap_or(0);
    let skip = (mine + 1).saturating_sub(max);
    list.iter().copied().enumerate().skip(skip).take(max).collect()
}

fn name_style(p: &Player, me: u64) -> Style {
    if p.id == me { bold(ACCENT) } else { fg(TEXT) }
}

fn lobby(buf: &mut Buffer, area: Rect, g: &Game, me: u64, start: Option<Instant>, now: Instant) {
    let mut y = logo(buf, area);
    let ready = g.player(me).is_some_and(|p| p.ready);
    let alone = g.players.iter().filter(|p| p.ready).count() == 1 && ready;
    let status = match start {
        Some(t) => Line::styled(format!("Départ dans {}", secs_left(t, now)), bold(GOOD)),
        None => Line::styled("En attente d'un adversaire...", fg(DIM)),
    };
    center(buf, area, y, status);
    y += 1;
    rule(buf, area, y);
    y += 1;

    let half = area.width / 2;
    let left = Rect { width: half - 1, ..area };
    let right = Rect { x: area.x + half + 1, width: area.width - half - 1, ..area };
    put(buf, left, y, Line::styled(format!("Joueurs ({}/{MAX_RACERS})", g.ready_count().min(MAX_RACERS)), bold(TEXT)));
    put(buf, right, y, Line::styled("Records du serveur", bold(TEXT)));
    y += 1;

    let rows = (area.bottom() - 2).saturating_sub(y) as usize;
    let all: Vec<&Player> = g.players.iter().collect();
    for (i, (_, p)) in window(&all, me, rows).into_iter().enumerate() {
        let queued = p.ready && !g.in_grid(p.id);
        let (mark, c) = if queued { ("… ", ACCENT) } else if p.ready { ("◆ ", GOOD) } else { ("◇ ", DIM) };
        let mut spans = vec![
            Span::styled(mark, fg(c)),
            Span::styled(format!("{:<17}", p.name), name_style(p, me)),
        ];
        if queued {
            spans.push(Span::styled("en file", fg(DIM)));
        } else if p.wins > 0 {
            spans.push(Span::styled(format!("{} vict.", p.wins), fg(DIM)));
        }
        put(buf, left, y + i as u16, Line::from(spans));
    }
    if g.top.is_empty() {
        put(buf, right, y, Line::styled("aucun pour l'instant", fg(DIM)));
    }
    for (i, (name, wpm)) in g.top.iter().enumerate().take(rows) {
        let line = Line::from(vec![
            Span::styled(format!("{}. ", i + 1), fg(DIM)),
            Span::styled(format!("{name:<17}"), fg(TEXT)),
            Span::styled(format!("{wpm} mpm"), fg(ACCENT)),
        ]);
        put(buf, right, y + i as u16, line);
    }
    let label = match (alone && start.is_none(), ready) {
        (true, _) => "courir seul",
        (_, true) => "pas prêt",
        (_, false) => "prêt",
    };
    footer(buf, area, &[("Entrée", label)]);
}

/// Coupe le texte sur les espaces ; renvoie des intervalles [début, fin).
pub fn wrap(text: &str, width: usize) -> Vec<(usize, usize)> {
    let b = text.as_bytes();
    let (mut out, mut start) = (Vec::new(), 0);
    while b.len() - start > width {
        let cut = (start + 1..start + width).rev().find(|&i| b[i] == b' ').map_or(start + width, |i| i + 1);
        out.push((start, cut));
        start = cut;
    }
    out.push((start, b.len()));
    out
}

fn race(buf: &mut Buffer, area: Rect, g: &Game, me: u64, start: Instant, ends: Instant, now: Instant) {
    let left = secs_left(ends, now);
    let color = if left <= 20 { BAD } else { DIM };
    let mut y = header(buf, area, "COURSE", format!("{left}s"), color);
    let mine = g.player(me).and_then(|p| p.run.as_ref());

    let text = g.text();
    for (s, e) in wrap(text, area.width as usize) {
        let spans: Vec<Span> = text[s..e]
            .char_indices()
            .map(|(i, c)| {
                let style = match mine {
                    Some(r) if s + i < r.pos => fg(GOOD),
                    Some(r) if s + i == r.pos => {
                        let bg = if r.wrong { BAD } else { ACCENT };
                        Style::new().fg(Color::Indexed(234)).bg(bg)
                    }
                    Some(_) => fg(DIM),
                    None => fg(TEXT),
                };
                Span::styled(c.to_string(), style)
            })
            .collect();
        put(buf, area, y, Line::from(spans));
        y += 1;
    }
    y += 1;
    rule(buf, area, y);
    y += 1;

    let elapsed = now.saturating_duration_since(start);
    let rows = (area.bottom() - 1).saturating_sub(y) as usize;
    for (i, (rank, p)) in window(&g.ranking(), me, rows).into_iter().enumerate() {
        let r = p.run.as_ref().unwrap();
        let bar = (area.width as usize).saturating_sub(32);
        let filled = r.pos * bar / text.len();
        let c = if r.finish.is_some() { GOOD } else { ACCENT };
        let line = Line::from(vec![
            Span::styled(format!("{:>2} ", rank + 1), fg(DIM)),
            Span::styled(format!("{:<17}", p.name), name_style(p, me)),
            Span::styled("█".repeat(filled), fg(c)),
            Span::styled("░".repeat(bar - filled), fg(TRACK)),
            Span::styled(format!("{:>4} mpm", r.wpm(elapsed)), fg(TEXT)),
        ]);
        put(buf, area, y + i as u16, line);
    }
    let hint = match mine {
        None => "spectateur : tu cours à la prochaine",
        Some(r) if r.finish.is_some() => "arrivé ! attente des autres",
        Some(r) if r.wrong => "mauvaise touche : tape la bonne pour avancer",
        Some(_) => "tape le texte",
    };
    footer(buf, area, &[("···", hint)]);
}

fn results(buf: &mut Buffer, area: Rect, g: &Game, me: u64, until: Instant, now: Instant) {
    let right = format!("salon dans {}", secs_left(until, now));
    let mut y = header(buf, area, "ARRIVÉE", right, DIM);
    let head = format!("{:>2} {:<17}{:>8}{:>9}{:>10}", "#", "joueur", "mpm", "préc.", "temps");
    put(buf, area, y, Line::styled(head, fg(DIM)));
    y += 1;
    let rows = (area.bottom() - 2).saturating_sub(y) as usize;
    for (i, (rank, p)) in window(&g.ranking(), me, rows).into_iter().enumerate() {
        let r = p.run.as_ref().unwrap();
        let time = match r.finish {
            Some(t) => format!("{:.1}s", t.as_secs_f32()),
            None => format!("{}%", r.pos * 100 / g.text().len()),
        };
        let wpm = r.wpm(g.race_time.max(Duration::from_secs(1)));
        let line = Line::from(vec![
            Span::styled(format!("{:>2} ", rank + 1), bold(if rank == 0 { ACCENT } else { DIM })),
            Span::styled(format!("{:<17}", p.name), name_style(p, me)),
            Span::styled(format!("{wpm:>8}"), bold(TEXT)),
            Span::styled(format!("{:>8}%", r.accuracy()), fg(TEXT)),
            Span::styled(format!("{time:>10}"), fg(if r.finish.is_some() { GOOD } else { BAD })),
        ]);
        put(buf, area, y + i as u16, line);
    }
    footer(buf, area, &[]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_covers_everything() {
        for t in crate::phrases::TEXTS {
            for w in [20, 56, 92] {
                let parts = wrap(t, w);
                assert!(parts.iter().all(|(s, e)| e - s <= w));
                assert_eq!(parts.iter().map(|&(s, e)| &t[s..e]).collect::<String>(), *t);
            }
        }
    }

    #[test]
    fn draws_at_any_size() {
        let now = Instant::now();
        let mut g = Game::new();
        for i in 0..30 {
            g.join(i, "ada", now);
        }
        for (w, h) in [(1, 1), (60, 18), (80, 24), (300, 100)] {
            let mut t = now;
            for step in [0, 10, 130, 8] {
                t += Duration::from_secs(step);
                g.tick(t);
                let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
                draw(&mut buf, View::Game(&g, 29), t);
                draw(&mut buf, View::Naming("x"), t);
            }
        }
    }
}
