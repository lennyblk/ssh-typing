//! Logique de course. Aucune E/S : le temps est toujours passé en paramètre.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::phrases;

pub const COUNTDOWN: Duration = Duration::from_secs(10);
pub const SOLO_COUNTDOWN: Duration = Duration::from_secs(3);
pub const RACE_LIMIT: Duration = Duration::from_secs(120);
/// Temps laissé aux autres après la première arrivée.
pub const AFTER_FIRST: Duration = Duration::from_secs(20);
pub const RESULTS: Duration = Duration::from_secs(8);
/// Coureurs par course ; les autres attendent leur tour.
pub const MAX_RACERS: usize = 8;
/// Temps minimal garanti à quelqu'un qui rejoint pendant le compte à rebours.
pub const JOIN_GRACE: Duration = Duration::from_secs(5);
/// Les arrivées ne peuvent pas repousser le départ au-delà.
pub const MAX_COUNTDOWN: Duration = Duration::from_secs(30);
/// Grille pleine : inutile d'attendre davantage.
pub const FULL_COUNTDOWN: Duration = Duration::from_secs(3);
pub const MAX_NAME: usize = 16;

pub const MAX_MESSAGE: usize = 120;
const CHAT_HISTORY: usize = 50;
/// Chat fermé avant le départ et juste après l'arrivée : les frappes de chat
/// ne doivent pas tomber dans la course, ni l'inverse.
const CHAT_LOCK: Duration = Duration::from_secs(3);
const CHAT_REOPEN: Duration = Duration::from_millis(1500);
/// Anti-flood : messages par seconde, et rafale tolérée.
const CHAT_RATE: f64 = 0.5;
const CHAT_BURST: f64 = 4.0;

/// Anti-collage : frappes soutenues par seconde, et rafale tolérée (gigue réseau).
const KEYS_PER_SEC: f64 = 30.0;
const KEY_BURST: f64 = 10.0;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Phase {
    Lobby { start_at: Option<Instant> },
    Racing { start: Instant, ends: Instant },
    Results { until: Instant },
}

#[derive(Clone)]
pub struct Run {
    pub pos: usize,
    pub errors: usize,
    pub wrong: bool,
    pub finish: Option<Duration>,
    tokens: f64,
    last_key: Instant,
}

impl Run {
    pub fn wpm(&self, elapsed: Duration) -> u32 {
        let t = self.finish.unwrap_or(elapsed).as_secs_f64().max(0.5);
        (self.pos as f64 / 5.0 * 60.0 / t).round() as u32
    }

    pub fn accuracy(&self) -> u32 {
        let total = self.pos + self.errors;
        if total == 0 { 100 } else { (self.pos * 100 / total) as u32 }
    }
}

#[derive(Clone)]
pub struct Player {
    pub id: u64,
    pub name: String,
    pub ready: bool,
    pub wins: u32,
    pub run: Option<Run>,
    chat_tokens: f64,
    last_chat: Instant,
}

#[derive(Clone)]
pub struct Message {
    pub from: u64,
    pub name: String,
    pub text: String,
}

#[derive(Clone)]
pub struct Game {
    pub phase: Phase,
    pub players: Vec<Player>,
    pub text: usize,
    /// Meilleurs scores depuis le démarrage du serveur.
    pub top: Vec<(String, u32)>,
    /// Durée de la course terminée, pour l'écran de résultats.
    pub race_time: Duration,
    pub chat: VecDeque<Message>,
    solo: bool,
    /// Départ au plus tard, fixé au début du compte à rebours.
    latest_start: Instant,
}

pub fn clean_name(raw: &str) -> String {
    raw.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(MAX_NAME)
        .collect()
}

/// Latin d'une colonne de large : ni contrôle, ni bidi, ni caractère combinant.
pub fn chat_char(c: char) -> bool {
    matches!(c, ' '..='~' | '\u{a1}'..='\u{ac}' | '\u{ae}'..='\u{24f}' | '€' | '’')
}

impl Game {
    pub fn new() -> Self {
        Game {
            phase: Phase::Lobby { start_at: None },
            players: Vec::new(),
            text: phrases::pick(None),
            top: Vec::new(),
            race_time: Duration::ZERO,
            chat: VecDeque::new(),
            solo: false,
            latest_start: Instant::now(),
        }
    }

    pub fn text(&self) -> &'static str {
        phrases::TEXTS[self.text]
    }

    pub fn player(&self, id: u64) -> Option<&Player> {
        self.players.iter().find(|p| p.id == id)
    }

    pub fn join(&mut self, id: u64, wanted: &str, now: Instant) {
        let base = match clean_name(wanted) {
            n if n.is_empty() => "errant".to_string(),
            n => n,
        };
        let mut name = base.clone();
        let mut n = 2;
        while self.players.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            let suffix = n.to_string();
            name = format!("{}{suffix}", &base[..base.len().min(MAX_NAME - suffix.len())]);
            n += 1;
        }
        self.players.push(Player {
            id,
            name,
            ready: true,
            wins: 0,
            run: None,
            chat_tokens: CHAT_BURST,
            last_chat: now,
        });
        self.tick(now);
        if let Phase::Lobby { start_at: Some(t) } = self.phase
            && self.in_grid(id)
            && self.ready_count() < MAX_RACERS
        {
            let t = t.max((now + JOIN_GRACE).min(self.latest_start));
            self.phase = Phase::Lobby { start_at: Some(t) };
        }
    }

    pub fn leave(&mut self, id: u64, now: Instant) {
        self.players.retain(|p| p.id != id);
        self.tick(now);
    }

    /// Entrée dans le salon : départ solo si seul prêt, sinon bascule prêt / pas prêt.
    pub fn press_enter(&mut self, id: u64, now: Instant) {
        if !matches!(self.phase, Phase::Lobby { .. }) {
            return;
        }
        let ready = self.ready_count();
        let Some(p) = self.players.iter_mut().find(|p| p.id == id) else { return };
        if p.ready && ready == 1 && !self.solo {
            self.solo = true;
        } else {
            p.ready = !p.ready;
        }
        self.tick(now);
    }

    pub fn chat_open(&self, now: Instant) -> bool {
        match self.phase {
            Phase::Lobby { start_at: None } => true,
            Phase::Lobby { start_at: Some(t) } => t.saturating_duration_since(now) > CHAT_LOCK,
            Phase::Racing { .. } => false,
            Phase::Results { until } => {
                until.saturating_duration_since(now) <= RESULTS - CHAT_REOPEN
            }
        }
    }

    /// Publie un message. Faux si le chat est fermé ou le joueur trop bavard.
    pub fn say(&mut self, id: u64, text: &str, now: Instant) -> bool {
        if !self.chat_open(now) {
            return false;
        }
        let Some(p) = self.players.iter_mut().find(|p| p.id == id) else { return false };
        let dt = now.saturating_duration_since(p.last_chat).as_secs_f64();
        p.last_chat = now;
        p.chat_tokens = (p.chat_tokens + dt * CHAT_RATE).min(CHAT_BURST);
        if p.chat_tokens < 1.0 {
            return false;
        }
        p.chat_tokens -= 1.0;
        let text = text.trim().chars().filter(|&c| chat_char(c)).take(MAX_MESSAGE).collect();
        self.chat.push_back(Message { from: id, name: p.name.clone(), text });
        if self.chat.len() > CHAT_HISTORY {
            self.chat.pop_front();
        }
        true
    }

    pub fn key(&mut self, id: u64, c: char, now: Instant) {
        let Phase::Racing { start, ends } = self.phase else { return };
        if now >= ends {
            return;
        }
        let text = self.text().as_bytes();
        let Some(p) = self.players.iter_mut().find(|p| p.id == id) else { return };
        let Some(run) = p.run.as_mut().filter(|r| r.finish.is_none()) else { return };

        let dt = now.saturating_duration_since(run.last_key).as_secs_f64();
        run.last_key = now;
        run.tokens = (run.tokens + dt * KEYS_PER_SEC).min(KEY_BURST);
        if run.tokens < 1.0 {
            return;
        }
        run.tokens -= 1.0;

        if text[run.pos] as char != c {
            run.errors += 1;
            run.wrong = true;
            return;
        }
        run.wrong = false;
        run.pos += 1;
        if run.pos == text.len() {
            let time = now - start;
            run.finish = Some(time);
            let (name, wpm) = (p.name.clone(), run.wpm(time));
            let winner = self
                .players
                .iter()
                .filter(|p| p.run.as_ref().is_some_and(|r| r.finish.is_some()))
                .count()
                == 1;
            if winner {
                self.phase = Phase::Racing { start, ends: ends.min(now + AFTER_FIRST) };
                if self.players.iter().filter(|p| p.run.is_some()).count() > 1 {
                    self.players.iter_mut().find(|p| p.id == id).unwrap().wins += 1;
                }
            }
            self.record(name, wpm);
        }
        self.tick(now);
    }

    fn record(&mut self, name: String, wpm: u32) {
        match self.top.iter_mut().find(|(n, _)| *n == name) {
            Some(e) => e.1 = e.1.max(wpm),
            None => self.top.push((name, wpm)),
        }
        self.top.sort_by_key(|e| std::cmp::Reverse(e.1));
        self.top.truncate(5);
    }

    /// Vrai si le joueur fait partie des premiers prêts de la file.
    pub fn in_grid(&self, id: u64) -> bool {
        self.players.iter().filter(|p| p.ready).take(MAX_RACERS).any(|p| p.id == id)
    }

    pub fn ready_count(&self) -> usize {
        self.players.iter().filter(|p| p.ready).count()
    }

    /// Fait avancer la machine à états. Idempotent.
    pub fn tick(&mut self, now: Instant) {
        match self.phase {
            Phase::Lobby { start_at } => {
                let ready = self.ready_count();
                if ready != 1 {
                    self.solo = false;
                }
                let can_start = ready >= 2 || self.solo;
                match start_at {
                    _ if !can_start => self.phase = Phase::Lobby { start_at: None },
                    None => {
                        let wait = if self.solo { SOLO_COUNTDOWN } else { COUNTDOWN };
                        self.latest_start = now + MAX_COUNTDOWN;
                        self.phase = Phase::Lobby { start_at: Some(now + wait) };
                    }
                    Some(t) if now >= t => self.start(now),
                    Some(t) if ready >= MAX_RACERS => {
                        let t = t.min(now + FULL_COUNTDOWN);
                        self.phase = Phase::Lobby { start_at: Some(t) };
                    }
                    Some(_) => {}
                }
            }
            Phase::Racing { start, ends } => {
                let done = self
                    .players
                    .iter()
                    .filter_map(|p| p.run.as_ref())
                    .all(|r| r.finish.is_some());
                if done || now >= ends {
                    self.race_time = now.min(ends) - start;
                    self.phase = Phase::Results { until: now + RESULTS };
                }
            }
            Phase::Results { until } => {
                if now >= until {
                    // Ceux qui attendaient passent devant ; tri stable.
                    self.players.sort_by_key(|p| p.run.is_some());
                    for p in &mut self.players {
                        // Aucune frappe pendant la course : joueur absent.
                        if let Some(r) = p.run.take() {
                            p.ready = r.pos + r.errors > 0;
                        }
                    }
                    self.solo = false;
                    self.phase = Phase::Lobby { start_at: None };
                    self.tick(now);
                }
            }
        }
    }

    fn start(&mut self, now: Instant) {
        self.text = phrases::pick(Some(self.text));
        let mut places = MAX_RACERS;
        for p in &mut self.players {
            let racing = p.ready && places > 0;
            places -= racing as usize;
            p.run = racing.then_some(Run {
                pos: 0,
                errors: 0,
                wrong: false,
                finish: None,
                tokens: KEY_BURST,
                last_key: now,
            });
        }
        self.solo = false;
        self.phase = Phase::Racing { start: now, ends: now + RACE_LIMIT };
    }

    /// Coureurs, du premier au dernier.
    pub fn ranking(&self) -> Vec<&Player> {
        let mut v: Vec<&Player> = self.players.iter().filter(|p| p.run.is_some()).collect();
        v.sort_by_key(|p| {
            let r = p.run.as_ref().unwrap();
            (r.finish.unwrap_or(Duration::MAX), std::cmp::Reverse(r.pos))
        });
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: u64) -> Duration {
        Duration::from_secs(s)
    }

    /// Deux joueurs, course lancée.
    fn racing() -> (Game, Instant) {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", t);
        g.join(2, "bob", t);
        let t = t + COUNTDOWN;
        g.tick(t);
        assert!(matches!(g.phase, Phase::Racing { .. }));
        (g, t)
    }

    fn type_all(g: &mut Game, id: u64, mut t: Instant) -> Instant {
        for c in g.text().chars() {
            t += Duration::from_millis(100);
            g.key(id, c, t);
        }
        t
    }

    #[test]
    fn texts_are_ascii() {
        assert!(phrases::TEXTS.iter().all(|t| t.chars().all(|c| (' '..='~').contains(&c))));
    }

    #[test]
    fn second_player_triggers_countdown() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", t);
        assert_eq!(g.phase, Phase::Lobby { start_at: None });
        g.join(2, "ada", t);
        assert_eq!(g.players[1].name, "ada2");
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + COUNTDOWN) });
        g.leave(2, t);
        assert_eq!(g.phase, Phase::Lobby { start_at: None });
    }

    #[test]
    fn solo_start() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", t);
        g.press_enter(1, t);
        g.tick(t + SOLO_COUNTDOWN);
        assert!(matches!(g.phase, Phase::Racing { .. }));
    }

    #[test]
    fn wrong_key_blocks() {
        let (mut g, t) = racing();
        g.key(1, '~', t + secs(1));
        let r = g.player(1).unwrap().run.as_ref().unwrap();
        assert_eq!((r.pos, r.errors, r.wrong), (0, 1, true));
    }

    #[test]
    fn full_race_then_absent_player_not_ready() {
        let (mut g, t) = racing();
        let t = type_all(&mut g, 1, t);
        assert_eq!(g.player(1).unwrap().wins, 1);
        assert_eq!(g.top[0].0, "ada");
        let Phase::Racing { ends, .. } = g.phase else { panic!() };
        assert_eq!(ends, t + AFTER_FIRST);
        g.tick(ends);
        assert!(matches!(g.phase, Phase::Results { .. }));
        assert_eq!(g.ranking()[0].id, 1);
        g.tick(ends + RESULTS);
        assert!(g.player(1).unwrap().ready && !g.player(2).unwrap().ready);
    }

    #[test]
    fn paste_is_throttled() {
        let (mut g, t) = racing();
        for c in g.text().chars() {
            g.key(1, c, t + secs(1));
        }
        assert!(g.player(1).unwrap().run.as_ref().unwrap().pos <= KEY_BURST as usize);
    }

    #[test]
    fn late_joiner_spectates_and_leavers_end_race() {
        let (mut g, t) = racing();
        g.join(3, "eve", t);
        g.key(3, 'a', t);
        assert!(g.player(3).unwrap().run.is_none());
        g.leave(1, t);
        g.leave(2, t);
        assert!(matches!(g.phase, Phase::Results { .. }));
    }

    #[test]
    fn joiners_are_added_and_extend_countdown_up_to_limit() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "a", t);
        g.join(2, "b", t);
        g.join(3, "c", t + secs(9));
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + secs(14)) });
        for (i, s) in [13, 17, 21, 25].into_iter().enumerate() {
            g.join(4 + i as u64, "d", t + secs(s));
        }
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + MAX_COUNTDOWN) });
        g.tick(t + MAX_COUNTDOWN);
        assert_eq!(g.ranking().len(), 7);
    }

    #[test]
    fn eight_racers_then_waiting_players_first() {
        let t = Instant::now();
        let mut g = Game::new();
        for id in 1..=10 {
            g.join(id, "a", t);
        }
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + FULL_COUNTDOWN) });
        let t = t + FULL_COUNTDOWN;
        g.tick(t);
        assert_eq!(g.ranking().len(), MAX_RACERS);
        assert!(g.player(9).unwrap().run.is_none());
        for id in 1..=8 {
            g.key(id, '~', t + secs(1));
        }
        g.tick(t + RACE_LIMIT);
        g.tick(t + RACE_LIMIT + RESULTS);
        assert!(g.in_grid(9) && g.in_grid(10) && !g.in_grid(8));
    }

    #[test]
    fn chat_closed_around_race_and_rate_limited() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", t);
        g.join(2, "bob", t);
        for _ in 0..4 {
            assert!(g.say(1, "h\u{1b}é\u{202e}", t));
        }
        assert_eq!(g.chat[0].text, "hé");
        assert!(!g.say(1, "flood", t));
        assert!(g.say(2, "ok", t));
        // Le chat ne décale pas le départ.
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + COUNTDOWN) });
        assert!(!g.say(2, "trop tard", t + secs(7)));
        g.tick(t + COUNTDOWN);
        assert!(!g.say(2, "course", t + secs(11)));
        let end = t + COUNTDOWN + RACE_LIMIT;
        g.tick(end);
        assert!(!g.chat_open(end + secs(1)));
        assert!(g.say(2, "gg", end + secs(2)));
        for i in 0..200 {
            g.say(2, "x", end + secs(2));
            g.tick(end + RESULTS);
            g.say(2, "x", end + RESULTS + secs(3 * i));
        }
        assert!(g.chat.len() <= CHAT_HISTORY);
    }
}
