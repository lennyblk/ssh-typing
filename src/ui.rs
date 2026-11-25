//! Rendu. Palette 256 couleurs : COLORTERM ne traverse pas SSH.

use std::time::{Duration, Instant};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Paragraph, Widget},
};

use crate::accounts::Stats;
use crate::boss::{self, Event, Fight, Fighter};
use crate::game::{CHAT_COLORS, Game, Mode, POINTS_TO_WIN, Phase, Player, Team};

const ACCENT: Color = Color::Indexed(215);
const DIM: Color = Color::Indexed(245);
const HAIRLINE: Color = Color::Indexed(239);
const TRACK: Color = Color::Indexed(237);
const KEY_BG: Color = Color::Indexed(238);
const TEXT: Color = Color::Indexed(252);
const GOOD: Color = Color::Indexed(114);
const MODE_SOLO: Color = Color::Indexed(186);
const MODE_TEAMS: Color = Color::Indexed(177);
const MODE_BOSS: Color = Color::Indexed(167);
const BAD: Color = Color::Indexed(203);

/// Une couleur par personne dans le chat, les plus contrastées d'abord.
const CHAT_PALETTE: [Color; CHAT_COLORS] = {
    const I: [u8; CHAT_COLORS] =
        [215, 81, 114, 213, 228, 203, 141, 43, 208, 111, 155, 175, 180, 87, 99, 197];
    let mut out = [Color::Reset; CHAT_COLORS];
    let mut i = 0;
    while i < CHAT_COLORS {
        out[i] = Color::Indexed(I[i]);
        i += 1;
    }
    out
};

pub const MIN_W: u16 = 60;
pub const MIN_H: u16 = 18;

const LOGO: [&str; 3] = [
    "█▀▀ █▀▀ █ █   ▀█▀ █ █ █▀█ ▀█▀ █▄ █ █▀▀",
    "▀▀█ ▀▀█ █▀█    █  ▀█▀ █▀▀  █  █ ▀█ █ █",
    "▀▀▀ ▀▀▀ ▀ ▀    ▀   ▀  ▀   ▀▀▀ ▀  ▀ ▀▀▀",
];

pub enum View<'a> {
    Naming(&'a str, &'a str),
    Game(&'a Game, u64, &'a str, Option<&'a Panel>),
}

/// Fiche de stats ; `stats` vide si le compte n'existe pas.
pub struct Panel {
    pub name: String,
    pub stats: Option<Stats>,
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
        View::Naming(name, error) => naming(buf, area, name, error),
        View::Game(_, _, _, Some(p)) => panel(buf, area, p),
        View::Game(g, me, draft, None) => match g.phase {
            Phase::Lobby { start_at } => lobby(buf, area, g, me, draft, start_at, now),
            Phase::Picking { until, last } => picking(buf, area, g, me, draft, until, last, now),
            Phase::Racing { start, ends } => race(buf, area, g, me, start, ends, now),
            Phase::Briefing { until } => briefing(buf, area, until, now),
            Phase::Boss => boss_fight(buf, area, g, me, now),
            Phase::Results { until } if g.mode == Mode::Boss => {
                boss_results(buf, area, g, me, draft, until, now)
            }
            Phase::Results { until } => results(buf, area, g, me, draft, until, now),
        },
    }
}

fn footer(buf: &mut Buffer, area: Rect, keys: &[(&str, &str)]) {
    let keys: Vec<_> = keys.iter().chain(&[("Ctrl+C", "quitter")]).collect();
    let len = |short: bool| -> usize {
        keys.iter()
            .map(|(k, l)| {
                let k = if short { k.replace("Ctrl+", "^") } else { k.to_string() };
                k.chars().count() + l.chars().count() + if short { 4 } else { 6 }
            })
            .sum()
    };
    let short = len(false) > area.width as usize;
    let mut spans = Vec::new();
    for (k, label) in keys {
        let k = if short { k.replace("Ctrl+", "^") } else { k.to_string() };
        let gap = if short { " " } else { "   " };
        spans.push(Span::styled(format!(" {k} "), bold(TEXT).bg(KEY_BG)));
        spans.push(Span::styled(format!(" {label}{gap}"), fg(DIM)));
    }
    center(buf, area, area.bottom() - 1, Line::from(spans));
}

fn team_color(t: Team) -> Color {
    match t {
        Team::Fox => Color::Indexed(208),
        Team::Falco => Color::Indexed(81),
    }
}

fn team_name(t: Team) -> &'static str {
    match t {
        Team::Fox => "FOX",
        Team::Falco => "FALCO",
    }
}

/// Score des deux équipes, avec une barre partagée entre elles.
fn team_line(buf: &mut Buffer, area: Rect, y: u16, g: &Game, elapsed: Duration) {
    let a = g.team_score(Team::Fox, elapsed);
    let b = g.team_score(Team::Falco, elapsed);
    let left = format!("FOX {a:.1} ");
    let right = format!(" {b:.1} FALCO");
    let bar = (area.width as usize).saturating_sub(left.len() + right.len());
    let share = if a + b > 0.0 { a / (a + b) } else { 0.5 };
    let filled = ((bar as f64 * share).round() as usize).min(bar);
    let (ca, cb) = (team_color(Team::Fox), team_color(Team::Falco));
    put(
        buf,
        area,
        y,
        Line::from(vec![
            Span::styled(left, bold(ca)),
            Span::styled("━".repeat(filled), fg(ca)),
            Span::styled("━".repeat(bar - filled), fg(cb)),
            Span::styled(right, bold(cb)),
        ]),
    );
}

#[allow(clippy::too_many_arguments)]
fn picking(
    buf: &mut Buffer,
    area: Rect,
    g: &Game,
    me: u64,
    draft: &str,
    until: Instant,
    last: bool,
    now: Instant,
) {
    let left = secs_left(until, now);
    let color = if left <= 5 { BAD } else { GOOD };
    let y = header(buf, area, "CHOISIS TON CAMP", format!("départ dans {left}"), color) - 1;
    let note = if last {
        Line::styled("il faut au moins un joueur de chaque côté !", bold(BAD))
    } else {
        Line::styled("2 points pour gagner · 1 phrase = 1 point", fg(DIM))
    };
    center(buf, area, y, note);
    let y = y + 1;

    let biggest = g.players.len().max(4) + 2;
    let height = list_rows(area, y, biggest) as u16;
    let w = area.width / 3;
    let mine = g.player(me).and_then(|p| p.team);
    let columns = [(Some(Team::Fox), 0), (None, 1), (Some(Team::Falco), 2)];
    for (team, i) in columns {
        let width = if i == 1 { area.width - 2 * w } else { w };
        let x = area.x + if i == 2 { area.width - w } else { i * w };
        let rect = Rect { x, y, width, height };
        let list: Vec<&Player> = g.players.iter().filter(|p| p.team == team && (p.ready || team.is_some())).collect();
        let inner = match team {
            None => {
                let inner = Rect { y: rect.y + 1, height: rect.height.saturating_sub(1), ..rect };
                center(buf, rect, rect.y, Line::styled(format!("◀ indécis ({}) ▶", list.len()), fg(DIM)));
                inner
            }
            Some(t) => {
                let chosen = mine == team;
                let c = team_color(t);
                let title = format!(" {} ({}) ", team_name(t), list.len());
                let block = Block::bordered()
                    .border_type(if chosen { BorderType::Thick } else { BorderType::Rounded })
                    .border_style(fg(if chosen { c } else { HAIRLINE }))
                    .title_top(Line::styled(title, bold(c)).centered());
                let inner = block.inner(rect);
                block.render(rect, buf);
                inner
            }
        };
        for (row, (_, p)) in window(&list, me, inner.height as usize).into_iter().enumerate() {
            let style = match (p.id == me, team) {
                (true, Some(t)) => bold(team_color(t)),
                (true, None) => bold(ACCENT),
                (false, Some(_)) => fg(TEXT),
                (false, None) => fg(DIM),
            };
            center(buf, inner, inner.y + row as u16, Line::styled(p.name.clone(), style));
        }
    }
    chat(buf, area, y + height, g, draft, now);
    footer(buf, area, &[("←", "Fox"), ("→", "Falco")]);
}

/// Points du match, un rond par point à gagner.
fn points_line(g: &Game) -> Line<'static> {
    let dots = |n: u32| {
        let full = n.min(POINTS_TO_WIN) as usize;
        format!("{}{}", "●".repeat(full), "○".repeat(POINTS_TO_WIN as usize - full))
    };
    let (c, b) = (team_color(Team::Fox), team_color(Team::Falco));
    Line::from(vec![
        Span::styled("FOX ", bold(c)),
        Span::styled(dots(g.teams.fox), bold(c)),
        Span::styled(format!("  {} - {}  ", g.teams.fox, g.teams.falco), bold(TEXT)),
        Span::styled(dots(g.teams.falco), bold(b)),
        Span::styled(" FALCO", bold(b)),
    ])
}

fn panel(buf: &mut Buffer, area: Rect, p: &Panel) {
    let y = header(buf, area, &format!("PROFIL · {}", p.name), String::new(), DIM);
    match &p.stats {
        None => put(buf, area, y, Line::styled("Personne n'a encore couru sous ce pseudo.", fg(TEXT))),
        Some(s) => {
            let opt = |v: Option<u32>, unit: &str| v.map_or("-".into(), |v| format!("{v}{unit}"));
            let rows = [
                ("Meilleur", format!("{} mpm", s.best)),
                ("Moyenne", opt(s.average(), " mpm")),
                ("Précision", opt(s.accuracy(), "%")),
                ("Courses terminées", s.races.to_string()),
                ("Victoires", s.wins.to_string()),
                ("Victoires en équipe", s.team_wins.to_string()),
                ("Boss vaincus", format!("{} sur {}", s.boss_wins, s.boss_fights)),
                ("Boss le plus rapide", match s.boss_best {
                    0 => "-".into(),
                    n => format!("{n}s"),
                }),
            ];
            for (i, (label, value)) in rows.into_iter().enumerate() {
                let line = Line::from(vec![
                    Span::styled(format!("{label:<21}"), fg(DIM)),
                    Span::styled(value, bold(TEXT)),
                ]);
                put(buf, area, y + i as u16, line);
            }
            if let (Some(min), Some(max)) = (s.history.iter().min(), s.history.iter().max()) {
                let spark: String = s
                    .history
                    .iter()
                    .map(|v| {
                        let level = (v - min) * 7 / (max - min).max(1);
                        ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'][level as usize]
                    })
                    .collect();
                let line = Line::from(vec![
                    Span::styled(format!("{:<21}", "Dernières courses"), fg(DIM)),
                    Span::styled(spark, fg(ACCENT)),
                    Span::styled(format!("  {min} à {max} mpm"), fg(DIM)),
                ]);
                put(buf, area, y + 9, line);
            }
        }
    }
    footer(buf, area, &[("Entrée", "fermer")]);
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

fn naming(buf: &mut Buffer, area: Rect, name: &str, error: &str) {
    let y = logo(buf, area);
    center(buf, area, y, Line::styled("course de frappe sous la cendre", fg(DIM)));
    let line = Line::from(vec![
        Span::styled("Ton pseudo  ", fg(DIM)),
        Span::styled(name.to_string(), bold(TEXT)),
        Span::styled("█", fg(ACCENT)),
    ]);
    center(buf, area, y + 3, line);
    center(buf, area, y + 4, Line::styled(error.to_string(), fg(BAD)));
    let hint = "reprends toujours le même pour garder tes stats";
    center(buf, area, y + 6, Line::styled(hint, fg(DIM)));
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

/// Hauteur de la liste : laisse au chat un filet, quelques messages et la saisie.
fn list_rows(area: Rect, y: u16, wanted: usize) -> usize {
    let avail = (area.bottom() - 2).saturating_sub(y) as usize;
    let chat = if avail >= 13 { 5 } else { 4 };
    wanted.min(avail.saturating_sub(chat))
}

fn chat(buf: &mut Buffer, area: Rect, y: u16, g: &Game, draft: &str, now: Instant) {
    rule(buf, area, y);
    let input = area.bottom() - 2;
    let width = area.width as usize;

    let mut lines = Vec::new();
    for m in &g.chat {
        let head = m.name.len() + 2;
        let body: Vec<char> = m.text.chars().collect();
        for (i, (s, e)) in wrap(&body, width - head).into_iter().enumerate() {
            let name = if i == 0 { format!("{}: ", m.name) } else { " ".repeat(head) };
            let color = CHAT_PALETTE[m.color % CHAT_COLORS];
            lines.push(Line::from(vec![
                Span::styled(name, bold(color)),
                Span::styled(body[s..e].iter().collect::<String>(), fg(TEXT)),
            ]));
        }
    }
    let rows = input.saturating_sub(y + 1) as usize;
    let skip = lines.len().saturating_sub(rows);
    for (i, l) in lines.into_iter().skip(skip).enumerate() {
        put(buf, area, y + 1 + i as u16, l);
    }

    let line = if !g.chat_open(now) {
        Line::styled("> chat fermé : mains sur le clavier", fg(DIM))
    } else if draft.is_empty() {
        Line::from(vec![
            Span::styled("> ", fg(ACCENT)),
            Span::styled("█", fg(ACCENT)),
            Span::styled(" écris un message", fg(DIM)),
        ])
    } else {
        let n = draft.chars().count();
        let tail: String = draft.chars().skip(n.saturating_sub(width - 3)).collect();
        Line::from(vec![
            Span::styled("> ", fg(ACCENT)),
            Span::styled(tail, fg(TEXT)),
            Span::styled("█", fg(ACCENT)),
        ])
    };
    put(buf, area, input, line);
}

/// Les modes côte à côte : celui qui l'emporte est plein, les autres éteints.
fn mode_cards(buf: &mut Buffer, area: Rect, g: &Game, me: u64) {
    let votes = g.votes();
    let mine = g.player(me).map(|p| p.vote);
    let third = area.width / 3;
    let cards = [
        (Mode::Solo, "INDIVIDUEL", MODE_SOLO),
        (Mode::Teams, "ÉQUIPES", MODE_TEAMS),
        (Mode::Boss, "BOSS", MODE_BOSS),
    ];
    for (i, (mode, label, color)) in cards.into_iter().enumerate() {
        let votes = votes[mode as usize];
        let x = area.x + i as u16 * third;
        let width = if i == 2 { area.right() - x } else { third };
        let rect = Rect { x, width, ..area };
        let leads = g.next_mode() == mode;
        let border = if leads { BorderType::Thick } else { BorderType::Rounded };
        let mut block = Block::bordered()
            .border_type(border)
            .border_style(fg(if leads { color } else { HAIRLINE }));
        if leads {
            block = block.title_top(Line::styled(" prochain mode ", bold(color)).centered());
        }
        if mine == Some(mode) {
            block = block.title_bottom(Line::styled(" ▲ ton vote ", bold(color)).centered());
        }
        let inner = block.inner(rect);
        block.render(rect, buf);
        let style = if leads { Style::new().fg(Color::Indexed(234)).bg(color) } else { fg(color) };
        if leads {
            buf.set_style(inner, style);
        }
        let text = format!("{label} · {votes}");
        center(buf, inner, inner.y, Line::styled(text, style.add_modifier(Modifier::BOLD)));
    }
}

fn lobby(
    buf: &mut Buffer,
    area: Rect,
    g: &Game,
    me: u64,
    draft: &str,
    start: Option<Instant>,
    now: Instant,
) {
    // Petit terminal : le choix du mode passe avant le logo.
    let mut y = if area.height >= 22 { logo(buf, area) - 1 } else { area.y };
    mode_cards(buf, Rect { y, height: 3, ..area }, g, me);
    y += 3;
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
    put(buf, left, y, Line::styled(format!("Joueurs ({} prêts sur {})", g.ready_count(), g.players.len()), bold(TEXT)));
    put(buf, right, y, Line::styled("Records du serveur", bold(TEXT)));
    y += 1;

    let all: Vec<&Player> = g.players.iter().collect();
    let rows = list_rows(area, y, all.len().max(g.top.len()).max(1));
    for (i, (_, p)) in window(&all, me, rows).into_iter().enumerate() {
        let (mark, c) = if p.ready { ("◆ ", GOOD) } else { ("◇ ", DIM) };
        let mut spans = vec![
            Span::styled(mark, fg(c)),
            Span::styled(format!("{:<17}", p.name), name_style(p, me)),
        ];
        if p.wins > 0 {
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
    chat(buf, area, y + rows as u16, g, draft, now);
    let label = match (alone && start.is_none(), ready) {
        _ if !draft.trim().is_empty() => "envoyer",
        (true, _) => "jouer seul",
        (_, true) => "pas prêt",
        (_, false) => "prêt",
    };
    footer(buf, area, &[("Entrée", label), ("← →", "mode"), ("Ctrl+P", "profil")]);
}

/// Coupe le texte sur les espaces ; renvoie des intervalles [début, fin).
pub fn wrap(b: &[char], width: usize) -> Vec<(usize, usize)> {
    let (mut out, mut start) = (Vec::new(), 0);
    while b.len() - start > width {
        let cut = (start + 1..start + width).rev().find(|&i| b[i] == ' ').map_or(start + width, |i| i + 1);
        out.push((start, cut));
        start = cut;
    }
    out.push((start, b.len()));
    out
}

fn race(buf: &mut Buffer, area: Rect, g: &Game, me: u64, start: Instant, ends: Instant, now: Instant) {
    let left = secs_left(ends, now);
    let ranking = g.ranking();
    let title = match g.mode {
        Mode::Solo => format!("COURSE · {} en piste", ranking.len()),
        _ => format!("PHRASE {}", g.teams.round),
    };
    let color = if left <= 20 { BAD } else { DIM };
    let mut y = header(buf, area, &title, format!("{left}s"), color);
    if g.mode == Mode::Teams {
        center(buf, area, area.y, points_line(g));
    }
    let mine = g.player(me).and_then(|p| p.run.as_ref());

    let text = g.text();
    let chars: Vec<char> = text.chars().collect();
    for (s, e) in wrap(&chars, area.width as usize) {
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
    if g.mode == Mode::Teams {
        team_line(buf, area, y, g, elapsed);
        y += 1;
    }
    let rows = (area.bottom() - 1).saturating_sub(y) as usize;
    for (i, (rank, p)) in window(&ranking, me, rows).into_iter().enumerate() {
        let r = p.run.as_ref().unwrap();
        let bar = (area.width as usize).saturating_sub(32);
        let filled = r.pos * bar / text.len();
        let c = match p.team {
            Some(t) => team_color(t),
            None if r.finish.is_some() => GOOD,
            None => ACCENT,
        };
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
        None if g.mode == Mode::Teams => "spectateur : ← → pour un camp à la fin de la phrase",
        None => "spectateur : tu cours à la prochaine",
        Some(r) if r.finish.is_some() => "arrivé ! attente des autres",
        Some(r) if r.wrong => "mauvaise touche : tape la bonne pour avancer",
        Some(_) => "tape le texte",
    };
    footer(buf, area, &[("···", hint)]);
}

fn results(
    buf: &mut Buffer,
    area: Rect,
    g: &Game,
    me: u64,
    draft: &str,
    until: Instant,
    now: Instant,
) {
    let next = g.match_on();
    let what = if next { "phrase suivante" } else { "salon" };
    let right = format!("{what} dans {}", secs_left(until, now));
    let title = match (g.mode, next) {
        (Mode::Solo, _) => "ARRIVÉE",
        (_, true) => "MANCHE",
        (_, false) => "FIN DU MATCH",
    };
    let mut y = header(buf, area, title, right, DIM);
    if g.mode == Mode::Teams {
        center(buf, area, area.y, points_line(g));
        let line = match (next, g.teams.round_winner, g.teams.champion) {
            (true, Some(t), _) => Line::styled(format!("Point pour {} !", team_name(t)), bold(team_color(t))),
            (true, None, _) => Line::styled("Manche nulle, aucun point", bold(TEXT)),
            (false, _, Some(t)) => {
                Line::styled(format!("★ {} remporte le match ! ★", team_name(t)), bold(team_color(t)))
            }
            (false, _, None) => Line::styled("Match nul", bold(TEXT)),
        };
        center(buf, area, y - 1, line);
        team_line(buf, area, y, g, g.race_time);
        y += 1;
    }
    let head = format!("{:>2} {:<17}{:>8}{:>9}{:>10}", "#", "joueur", "mpm", "préc.", "temps");
    put(buf, area, y, Line::styled(head, fg(DIM)));
    y += 1;
    let ranking = g.ranking();
    let rows = list_rows(area, y, ranking.len());
    for (i, (rank, p)) in window(&ranking, me, rows).into_iter().enumerate() {
        let r = p.run.as_ref().unwrap();
        let time = match r.finish {
            Some(t) => format!("{:.1}s", t.as_secs_f32()),
            None => format!("{}%", r.pos * 100 / g.text().len()),
        };
        let wpm = r.wpm(g.race_time.max(Duration::from_secs(1)));
        let line = Line::from(vec![
            Span::styled(
                format!("{:>2} ", rank + 1),
                bold(match p.team {
                    Some(t) => team_color(t),
                    None if rank == 0 => ACCENT,
                    None => DIM,
                }),
            ),
            Span::styled(format!("{:<17}", p.name), name_style(p, me)),
            Span::styled(format!("{wpm:>8}"), bold(TEXT)),
            Span::styled(format!("{:>8}%", r.accuracy()), fg(TEXT)),
            Span::styled(format!("{time:>10}"), fg(if r.finish.is_some() { GOOD } else { BAD })),
        ]);
        put(buf, area, y + i as u16, line);
    }
    chat(buf, area, y + rows as u16, g, draft, now);
    if next && g.player(me).is_some_and(|p| p.team.is_none()) {
        footer(buf, area, &[("←", "Fox"), ("→", "Falco")]);
    } else {
        footer(buf, area, &[("Entrée", "envoyer")]);
    }
}

// ─── BOSS ────────────────────────────────────────────────────────────────────

const INK: Color = Color::Indexed(234);
const GO: Color = Color::Indexed(71);
const WARN: Color = Color::Indexed(179);
const DANGER: Color = Color::Indexed(160);

/// `@` : yeux, d'une autre couleur.
const COLOSSUS: [&str; 6] = [
    "      ▄▄██████████▄▄      ",
    "   ▄██████████████████▄   ",
    "  ████  @@@████@@@  ████  ",
    "  █████▄   ████   ▄█████  ",
    "   ▀███▄▀▄▀▄▀▄▀▄▀▄▀███▀   ",
    " ▄███▀▀████████████▀▀███▄ ",
];

/// Consigne du moment : trois lignes pleines, impossibles à rater.
fn banner(buf: &mut Buffer, area: Rect, y: u16, text: &str, color: Color, ink: Color) {
    let rect = Rect { y, height: 3, ..area }.intersection(area);
    buf.set_style(rect, Style::new().bg(color));
    center(buf, area, y + 1, Line::styled(text.to_string(), bold(ink).bg(color)));
}

fn gauge(value: u32, max: u32, width: usize, color: Color) -> [Span<'static>; 2] {
    let mut full = (value as usize * width).div_ceil(max.max(1) as usize).min(width);
    if value == 0 {
        full = 0;
    }
    [Span::styled("█".repeat(full), fg(color)), Span::styled("░".repeat(width - full), fg(TRACK))]
}

fn hp_color(hp: i32) -> Color {
    match hp {
        60.. => GOOD,
        30.. => Color::Indexed(220),
        _ => BAD,
    }
}

fn stage_color(stage: u32) -> Color {
    [Color::Indexed(141), Color::Indexed(208), Color::Indexed(196)][stage as usize - 1]
}

fn briefing(buf: &mut Buffer, area: Rect, until: Instant, now: Instant) {
    let right = format!("combat dans {}", secs_left(until, now));
    let y = header(buf, area, boss::NAME, right, BAD) - 1;
    let white = Color::Indexed(231);
    let rules = [
        (GO, "TAPE LA PHRASE  =  TU FRAPPES   (mots parfaits : x2, x3)".to_string()),
        (WARN, format!("FAUTE DE FRAPPE  =  -{} PV ET COMBO PERDU", boss::FAULT_DAMAGE)),
        (DANGER, format!("MOT ROUGE  =  TAPE-LE VITE, SINON -{} PV", boss::MISS_DAMAGE)),
    ];
    for (i, (color, text)) in rules.iter().enumerate() {
        let ink = if *color == DANGER { white } else { INK };
        banner(buf, area, y + i as u16 * 3, text, *color, ink);
    }
    let last = "0 PV : à terre. Battez-le ensemble avant l'enrage !";
    center(buf, area, y + 10, Line::styled(last, bold(TEXT)));
    footer(buf, area, &[("···", "lis vite, ça arrive")]);
}

fn boss_fight(buf: &mut Buffer, area: Rect, g: &Game, me: u64, now: Instant) {
    let Some(f) = &g.fight else { return };
    let mine = f.fighter(me);
    let parry = mine.and_then(|m| m.parry.as_ref());
    let stage = stage_color(f.stage());
    let white = Color::Indexed(231);

    // Boss : nom, enrage, vie.
    let enrage = f.enrage_in(now).as_secs();
    let (timer, c) = match enrage {
        0 => ("ENRAGÉ !".to_string(), BAD),
        n => (format!("enrage dans {n}s"), if n <= 20 { BAD } else { DIM }),
    };
    let title = format!("{} · phase {}/3", boss::NAME, f.stage());
    put(buf, area, area.y, Line::styled(title, bold(stage)));
    put(buf, area, area.y, Line::styled(timer, bold(c)).right_aligned());
    let label = format!(" {:>3}%", (f.hp as u64 * 100).div_ceil(f.max as u64));
    let mut bar = gauge(f.hp, f.max, area.width as usize - label.len(), stage).to_vec();
    bar.push(Span::styled(label, bold(stage)));
    put(buf, area, area.y + 1, Line::from(bar));
    let mut y = area.y + 2;

    if area.height >= 22 {
        let eyes = if f.attacking() { Color::Indexed(196) } else { Color::Indexed(228) };
        for row in COLOSSUS {
            let spans: Vec<Span> = row
                .chars()
                .map(|c| match c {
                    '@' => Span::styled("█", fg(eyes)),
                    c => Span::styled(c.to_string(), fg(stage)),
                })
                .collect();
            center(buf, area, y, Line::from(spans));
            y += 1;
        }
    }

    // Consigne.
    match (mine, parry) {
        (None, _) => banner(buf, area, y, "SPECTATEUR : tu joues à la prochaine", TRACK, TEXT),
        (Some(m), _) if !m.alive() => {
            banner(buf, area, y, "À TERRE ! Tes alliés continuent", TRACK, BAD)
        }
        (Some(_), Some(p)) => {
            let text = format!("▶▶▶  PARE ! TAPE « {} »  ◀◀◀", p.word.to_uppercase());
            banner(buf, area, y, &text, DANGER, white);
        }
        (Some(m), None) if m.mult() > 1 => {
            let text = format!("FRAPPE ! TAPE LA PHRASE   COMBO x{}", m.mult());
            banner(buf, area, y, &text, if m.mult() == 2 { GO } else { Color::Indexed(214) }, INK);
        }
        (Some(_), None) => banner(buf, area, y, "FRAPPE ! TAPE LA PHRASE", GO, INK),
    }
    y += 3;

    // Ce qu'il faut taper.
    if let Some(p) = parry {
        let spans: Vec<Span> = p
            .word
            .chars()
            .enumerate()
            .map(|(i, c)| {
                let style = match i.cmp(&p.pos) {
                    std::cmp::Ordering::Less => bold(GOOD),
                    std::cmp::Ordering::Equal => bold(white).bg(DANGER),
                    std::cmp::Ordering::Greater => bold(BAD),
                };
                Span::styled(format!(" {c} "), style)
            })
            .collect();
        center(buf, area, y + 1, Line::from(spans));
        let left = p.until.saturating_duration_since(now).as_millis() as u32;
        let time = gauge(left, p.window.as_millis() as u32, area.width as usize, DANGER);
        put(buf, area, y + 2, Line::from(time.to_vec()));
    } else if let Some(m) = mine.filter(|m| m.alive()) {
        let text = f.text(m);
        let chars: Vec<char> = text.chars().collect();
        for (i, (s, e)) in wrap(&chars, area.width as usize).into_iter().enumerate() {
            let spans: Vec<Span> = text[s..e]
                .char_indices()
                .map(|(i, c)| {
                    let style = match (s + i).cmp(&m.pos) {
                        std::cmp::Ordering::Less => fg(GOOD),
                        std::cmp::Ordering::Equal => {
                            Style::new().fg(INK).bg(if m.wrong { BAD } else { ACCENT })
                        }
                        std::cmp::Ordering::Greater => fg(TEXT),
                    };
                    Span::styled(c.to_string(), style)
                })
                .collect();
            put(buf, area, y + i as u16, Line::from(spans));
        }
    }
    y += 4;

    // Moi : vie, combo, dernier événement.
    if let Some(m) = mine {
        let c = hp_color(m.hp);
        let hit = m.event.is_some_and(|(e, t)| {
            !matches!(e, Event::Hit { .. } | Event::Parried)
                && now.saturating_duration_since(t) < Duration::from_millis(500)
        });
        let mut spans = vec![Span::styled("TOI ", bold(if hit { BAD } else { ACCENT }))];
        spans.extend(gauge(m.hp as u32, boss::MAX_HP as u32, 18, if hit { white } else { c }));
        spans.push(Span::styled(format!(" {:>3} PV  ", m.hp), bold(c)));
        let dots = if m.mult() == boss::MAX_MULT { boss::COMBO_STEP } else { m.combo % boss::COMBO_STEP };
        spans.push(Span::styled(format!("x{} ", m.mult()), bold(Color::Indexed(214))));
        spans.push(Span::styled("●".repeat(dots as usize), fg(Color::Indexed(214))));
        spans.push(Span::styled("○".repeat((boss::COMBO_STEP - dots) as usize), fg(HAIRLINE)));
        put(buf, area, y, Line::from(spans));
        put(buf, area, y, event_line(m, now).right_aligned());
    }
    y += 1;
    rule(buf, area, y);
    y += 1;

    // Alliés, sur deux colonnes.
    let half = area.width / 2;
    let rows = (area.bottom() - 1).saturating_sub(y) as usize;
    let allies = f.fighters.iter().filter(|a| a.id != me).take(rows * 2);
    for (i, a) in allies.enumerate() {
        let col = Rect { x: area.x + (i % 2) as u16 * half, width: half, ..area };
        let c = hp_color(a.hp);
        let mut spans = vec![Span::styled(format!("{:<16} ", a.name), fg(if a.alive() { TEXT } else { DIM }))];
        if !a.alive() {
            spans.push(Span::styled("à terre", bold(BAD)));
        } else {
            spans.extend(gauge(a.hp as u32, boss::MAX_HP as u32, 8, c));
            if a.parry.is_some() {
                spans.push(Span::styled(" !", bold(BAD)));
            }
        }
        put(buf, col, y + (i / 2) as u16, Line::from(spans));
    }
    let hint = match (mine, parry) {
        (Some(_), Some(_)) => "tape le mot rouge !",
        (Some(m), None) if m.alive() && m.wrong => "mauvaise touche : tape la bonne pour avancer",
        (Some(m), None) if m.alive() => "chaque mot frappe le boss",
        _ => "regarde le combat",
    };
    footer(buf, area, &[("···", hint)]);
}

fn event_line(m: &Fighter, now: Instant) -> Line<'static> {
    let Some((event, at)) = m.event else { return Line::default() };
    if now.saturating_duration_since(at) > Duration::from_millis(1500) {
        return Line::default();
    }
    match event {
        Event::Hit { damage, mult } => {
            let c = [GOOD, GOOD, Color::Indexed(214)][mult as usize - 1];
            Line::styled(format!("+{damage} dégâts"), bold(c))
        }
        Event::Fault => Line::styled(format!("FAUTE -{} PV", boss::FAULT_DAMAGE), bold(BAD)),
        Event::Parried => Line::styled("PARÉ !", bold(Color::Indexed(81))),
        Event::Missed => Line::styled(format!("RATÉ -{} PV", boss::MISS_DAMAGE), bold(BAD)),
        Event::Enraged(n) => Line::styled(format!("ENRAGE -{n} PV"), bold(BAD)),
        Event::Down => Line::styled("À TERRE", bold(BAD)),
    }
}

fn boss_results(buf: &mut Buffer, area: Rect, g: &Game, me: u64, draft: &str, until: Instant, now: Instant) {
    let Some(f): Option<&Fight> = g.fight.as_ref() else { return };
    let right = format!("salon dans {}", secs_left(until, now));
    let y = header(buf, area, boss::NAME, right, DIM) - 1;
    if f.won == Some(true) {
        let text = format!("★  VICTOIRE en {}s  ★", f.time.as_secs());
        banner(buf, area, y, &text, GO, INK);
    } else {
        let left = (f.hp as u64 * 100).div_ceil(f.max as u64);
        let text = format!("DÉFAITE : il lui restait {left}%");
        banner(buf, area, y, &text, DANGER, Color::Indexed(231));
    }
    let mut y = y + 3;
    let head = format!("  {:<17}{:>8}{:>9}{:>8}{:>6}", "joueur", "dégâts", "parades", "fautes", "PV");
    put(buf, area, y, Line::styled(head, fg(DIM)));
    y += 1;
    let ranking = f.ranking();
    let rows = list_rows(area, y, ranking.len());
    let mine = ranking.iter().position(|p| p.id == me).unwrap_or(0);
    let skip = (mine + 1).saturating_sub(rows);
    for (i, (rank, p)) in ranking.iter().enumerate().skip(skip).take(rows).enumerate() {
        let mvp = rank == 0 && p.damage > 0;
        let line = Line::from(vec![
            Span::styled(if mvp { "★ " } else { "  " }, bold(Color::Indexed(214))),
            Span::styled(format!("{:<17}", p.name), if p.id == me { bold(ACCENT) } else { fg(TEXT) }),
            Span::styled(format!("{:>8}", p.damage), bold(GOOD)),
            Span::styled(format!("{:>9}", p.parries), fg(Color::Indexed(81))),
            Span::styled(format!("{:>8}", p.errors), fg(WARN)),
            Span::styled(format!("{:>6}", p.hp), bold(hp_color(p.hp))),
        ]);
        put(buf, area, y + i as u16, line);
    }
    chat(buf, area, y + rows as u16, g, draft, now);
    footer(buf, area, &[("Entrée", "envoyer")]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_covers_everything() {
        for t in crate::phrases::TEXTS {
            for w in [20, 56, 92] {
                let parts = wrap(&t.chars().collect::<Vec<_>>(), w);
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
            g.join(i, "ada", Stats::default(), now);
            g.cycle_vote(i, true);
        }
        for id in 0..30 {
            g.say(id, &"é".repeat(120), now);
        }
        for (w, h) in [(1, 1), (60, 18), (80, 24), (300, 100)] {
            let mut t = now;
            for step in [0, 30, 1, 15, 130, 8] {
                t += Duration::from_secs(step);
                g.tick(t);
                let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
                draw(&mut buf, View::Game(&g, 29, &"é".repeat(120), None), t);
                draw(&mut buf, View::Naming("x", "erreur"), t);
                let mut stats = Stats::default();
                for wpm in [0, 80, 400] {
                    let p = Panel { name: "x".into(), stats: Some(stats.clone()) };
                    draw(&mut buf, View::Game(&g, 29, "", Some(&p)), t);
                    stats.finished(wpm, 100);
                }
                draw(&mut buf, View::Game(&g, 29, "", Some(&Panel { name: "x".into(), stats: None })), t);
            }
        }
    }

    #[test]
    fn boss_draws_at_any_size() {
        for n in [1, 30] {
            let now = Instant::now();
            let mut g = Game::new();
            for i in 0..n {
                g.join(i, "abcdefghijklmnop", Stats::default(), now);
                g.cycle_vote(i, false);
            }
            g.press_enter(0, now);
            let mut t = now;
            for step in [3, 7, 7, 7, 2, 2, 4, 30, 30, 30, 30, 30] {
                t += Duration::from_secs(step);
                g.tick(t);
                g.key(0, 'e', t);
                for (w, h) in [(60, 18), (80, 24), (300, 100)] {
                    for me in [0, 99] {
                        let mut buf = Buffer::empty(Rect::new(0, 0, w, h));
                        draw(&mut buf, View::Game(&g, me, "", None), t);
                    }
                }
            }
        }
    }
}
