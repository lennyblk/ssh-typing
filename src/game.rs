//! Logique de course. Aucune E/S : le temps est toujours passé en paramètre.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::accounts::Stats;
use crate::boss::Fight;
use crate::phrases;

pub const COUNTDOWN: Duration = Duration::from_secs(10);
pub const SOLO_COUNTDOWN: Duration = Duration::from_secs(3);
pub const RACE_LIMIT: Duration = Duration::from_secs(120);
/// Temps laissé aux autres après la première arrivée.
pub const AFTER_FIRST: Duration = Duration::from_secs(20);
pub const RESULTS: Duration = Duration::from_secs(8);
/// Temps minimal garanti à quelqu'un qui rejoint pendant le compte à rebours.
pub const JOIN_GRACE: Duration = Duration::from_secs(5);
/// Les arrivées ne peuvent pas repousser le départ au-delà.
pub const MAX_COUNTDOWN: Duration = Duration::from_secs(30);
/// Lecture des règles avant le boss.
pub const BRIEFING: Duration = Duration::from_secs(7);
/// Choix du camp, avant un match en équipes.
pub const PICK: Duration = Duration::from_secs(15);
/// Tout le monde a choisi : inutile d'attendre davantage.
pub const PICK_DONE: Duration = Duration::from_secs(5);
/// Points pour gagner un match (BO3), et garde-fou si des manches sont nulles.
pub const POINTS_TO_WIN: u32 = 2;
const MAX_ROUNDS: u32 = 5;
pub const MAX_NAME: usize = 16;

pub const MAX_RECORDS: usize = 5;
pub const MAX_MESSAGE: usize = 120;
/// Taille de la palette du chat ; au-delà, les couleurs se répètent.
pub const CHAT_COLORS: usize = 16;
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
    /// Choix des camps. `last` : prolongation déjà accordée.
    Picking { until: Instant, last: bool },
    Racing { start: Instant, ends: Instant },
    Briefing { until: Instant },
    Boss,
    Results { until: Instant },
}

/// Plafond de vitesse de frappe, contre le collage.
#[derive(Clone)]
pub struct Throttle {
    tokens: f64,
    last: Instant,
}

impl Throttle {
    pub fn new(now: Instant) -> Self {
        Throttle { tokens: KEY_BURST, last: now }
    }

    pub fn allow(&mut self, now: Instant) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * KEYS_PER_SEC).min(KEY_BURST);
        let ok = self.tokens >= 1.0;
        if ok {
            self.tokens -= 1.0;
        }
        ok
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Solo,
    Teams,
    Boss,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Solo, Mode::Teams, Mode::Boss];
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Team {
    Fox,
    Falco,
}

impl Team {
    pub fn other(self) -> Team {
        match self {
            Team::Fox => Team::Falco,
            Team::Falco => Team::Fox,
        }
    }
}

/// Match en équipes en cours.
#[derive(Clone, Default)]
pub struct Match {
    pub fox: u32,
    pub falco: u32,
    /// Numéro de la manche en cours, à partir de 1.
    pub round: u32,
    /// Gagnant de la dernière manche, puis du match une fois `over`.
    pub round_winner: Option<Team>,
    pub champion: Option<Team>,
    pub over: bool,
}

#[derive(Clone)]
pub struct Run {
    pub pos: usize,
    pub errors: usize,
    pub wrong: bool,
    pub finish: Option<Duration>,
    keys: Throttle,
}

impl Run {
    pub fn wpm(&self, elapsed: Duration) -> u32 {
        let t = self.finish.unwrap_or(elapsed).as_secs_f64().max(0.5);
        (self.pos as f64 / 5.0 * 60.0 / t).round() as u32
    }

    /// A tapé au moins une touche.
    pub fn active(&self) -> bool {
        self.pos + self.errors > 0
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
    /// Indice de couleur dans le chat, libre parmi les connectés.
    pub color: usize,
    pub ready: bool,
    pub wins: u32,
    pub run: Option<Run>,
    pub vote: Mode,
    pub team: Option<Team>,
    /// A tapé pendant le match en cours.
    played: bool,
    pub stats: Arc<Stats>,
    chat_tokens: f64,
    last_chat: Instant,
}

#[derive(Clone)]
pub struct Message {
    pub color: usize,
    pub name: String,
    pub text: String,
}

#[derive(Clone)]
pub struct Game {
    pub phase: Phase,
    pub players: Vec<Player>,
    pub text: usize,
    /// Meilleurs scores, conservés entre deux démarrages (voir `records`).
    pub top: Vec<(String, u32)>,
    /// Durée de la course terminée, pour l'écran de résultats.
    pub race_time: Duration,
    pub chat: VecDeque<Message>,
    /// Mode de la course en cours ou terminée.
    pub mode: Mode,
    pub teams: Match,
    /// Combat en cours, ou terminé pendant l'écran de résultats.
    pub fight: Option<Fight>,
    /// Stats à reporter dans les comptes (voir `World::sync`).
    pub outbox: Vec<(String, Stats)>,
    /// Textes restant à jouer avant de remélanger.
    bag: Vec<usize>,
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
            text: 0,
            bag: phrases::shuffled(None),
            top: Vec::new(),
            race_time: Duration::ZERO,
            chat: VecDeque::new(),
            mode: Mode::Solo,
            teams: Match::default(),
            fight: None,
            outbox: Vec::new(),
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

    /// Le pseudo doit être propre et libre (voir `World::join`).
    pub fn join(&mut self, id: u64, name: &str, stats: Stats, now: Instant) {
        // Couleur la moins récemment vue dans le chat parmi celles qui sont libres.
        let color = (0..)
            .filter(|c| self.players.iter().all(|p| p.color != *c))
            .take(CHAT_COLORS.saturating_sub(self.players.len()).max(1))
            .min_by_key(|c| self.chat.iter().rposition(|m| m.color == *c).map_or(0, |i| i + 1))
            .unwrap();
        self.players.push(Player {
            id,
            name: name.to_string(),
            color,
            ready: true,
            wins: 0,
            run: None,
            vote: Mode::Solo,
            team: None,
            played: false,
            stats: Arc::new(stats),
            chat_tokens: CHAT_BURST,
            last_chat: now,
        });
        self.tick(now);
        if let Phase::Lobby { start_at: Some(t) } = self.phase {
            let t = t.max((now + JOIN_GRACE).min(self.latest_start));
            self.phase = Phase::Lobby { start_at: Some(t) };
        }
    }

    pub fn leave(&mut self, id: u64, now: Instant) {
        self.players.retain(|p| p.id != id);
        // Après le combat, la fiche reste pour l'écran de résultats.
        if self.phase == Phase::Boss
            && let Some(f) = &mut self.fight
        {
            f.leave(id);
        }
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

    /// Vote suivant (ou précédent) pour le mode de la prochaine partie.
    pub fn cycle_vote(&mut self, id: u64, forward: bool) {
        if !matches!(self.phase, Phase::Lobby { .. }) {
            return;
        }
        if let Some(p) = self.players.iter_mut().find(|p| p.id == id) {
            let n = Mode::ALL.len();
            let step = if forward { 1 } else { n - 1 };
            p.vote = Mode::ALL[(p.vote as usize + step) % n];
        }
    }

    /// Flèche gauche ou droite : mode dans le salon, camp ailleurs.
    pub fn arrow(&mut self, id: u64, right: bool, now: Instant) {
        if matches!(self.phase, Phase::Lobby { .. }) {
            self.cycle_vote(id, right);
        } else {
            self.choose(id, if right { Team::Falco } else { Team::Fox }, now);
        }
    }

    /// Votes des joueurs prêts, dans l'ordre de `Mode::ALL`.
    pub fn votes(&self) -> [usize; 3] {
        let mut votes = [0; 3];
        for p in self.players.iter().filter(|p| p.ready) {
            votes[p.vote as usize] += 1;
        }
        votes
    }

    /// Le plus voté l'emporte ; à égalité, le premier de `Mode::ALL`.
    pub fn next_mode(&self) -> Mode {
        let votes = self.votes();
        let possible = |m: &Mode| *m != Mode::Teams || self.ready_count() >= 2;
        let modes = Mode::ALL.into_iter().filter(possible);
        // min_by_key garde le premier en cas d'égalité.
        modes.min_by_key(|m| std::cmp::Reverse(votes[*m as usize])).unwrap_or(Mode::Solo)
    }

    fn start_fight(&mut self, now: Instant) {
        let players: Vec<_> = self
            .players
            .iter()
            .filter(|p| p.ready)
            .map(|p| {
                let level = match (p.stats.average(), p.stats.best) {
                    (Some(avg), _) => avg,
                    (None, 0) => 55,
                    (None, best) => best * 4 / 5,
                };
                (p.id, p.name.clone(), level)
            })
            .collect();
        self.fight = Some(Fight::new(&players, now));
        self.phase = Phase::Boss;
    }

    fn end_fight(&mut self, now: Instant) {
        let Some(fight) = &self.fight else { return };
        let won = fight.won == Some(true);
        let secs = fight.time.as_secs().max(1) as u32;
        let ids: Vec<u64> = fight.fighters.iter().filter(|f| f.active()).map(|f| f.id).collect();
        for id in ids {
            self.update_stats(id, |s| {
                s.boss_fights += 1;
                if won {
                    s.boss_wins += 1;
                    s.boss_best = if s.boss_best == 0 { secs } else { s.boss_best.min(secs) };
                }
            });
            if won && let Some(p) = self.players.iter_mut().find(|p| p.id == id) {
                p.wins += 1;
            }
        }
        self.phase = Phase::Results { until: now + RESULTS };
    }

    /// Match en équipes commencé et pas encore gagné.
    pub fn match_on(&self) -> bool {
        self.mode == Mode::Teams
            && !self.teams.over
            && matches!(self.phase, Phase::Racing { .. } | Phase::Results { .. })
    }

    /// Rejoint un camp. Entre deux manches, seulement pour qui n'en a pas.
    pub fn choose(&mut self, id: u64, team: Team, now: Instant) {
        let picking = matches!(self.phase, Phase::Picking { .. });
        let between = self.match_on() && matches!(self.phase, Phase::Results { .. });
        if let Some(p) = self.players.iter_mut().find(|p| p.id == id)
            && (picking || (between && p.team.is_none()))
        {
            p.team = Some(team);
            p.ready = true;
        }
        self.tick(now);
    }

    pub fn team_size(&self, team: Team) -> usize {
        self.players.iter().filter(|p| p.team == Some(team)).count()
    }

    fn empty_team(&self) -> Option<Team> {
        [Team::Fox, Team::Falco].into_iter().find(|t| self.team_size(*t) == 0)
    }

    fn end_picking(&mut self, last: bool, now: Instant) {
        // Les indécis complètent le camp le moins fourni.
        for i in 0..self.players.len() {
            if self.players[i].ready && self.players[i].team.is_none() {
                let (c, g) = (self.team_size(Team::Fox), self.team_size(Team::Falco));
                let fox = if c == g { rand::random() } else { c < g };
                self.players[i].team = Some(if fox { Team::Fox } else { Team::Falco });
            }
        }
        if self.empty_team().is_none() {
            self.teams = Match::default();
            self.start(Mode::Teams, now);
        } else if !last {
            self.phase = Phase::Picking { until: now + PICK, last: true };
        } else {
            self.clear_teams();
            self.start(Mode::Solo, now);
        }
    }

    fn clear_teams(&mut self) {
        for p in &mut self.players {
            p.team = None;
            p.played = false;
        }
    }

    /// Moyenne des mpm de l'équipe, sans compter les absents.
    pub fn team_score(&self, team: Team, elapsed: Duration) -> f64 {
        let runs = self.players.iter().filter(|p| p.team == Some(team));
        let wpm: Vec<f64> = runs
            .filter_map(|p| p.run.as_ref())
            .filter(|r| r.active())
            .map(|r| r.wpm(elapsed) as f64)
            .collect();
        if wpm.is_empty() { 0.0 } else { wpm.iter().sum::<f64>() / wpm.len() as f64 }
    }

    /// Modifie les stats d'un joueur et prévoit leur sauvegarde.
    fn update_stats(&mut self, id: u64, f: impl FnOnce(&mut Stats)) {
        if let Some(p) = self.players.iter_mut().find(|p| p.id == id) {
            f(Arc::make_mut(&mut p.stats));
            self.outbox.push((p.name.clone(), Stats::clone(&p.stats)));
        }
    }

    fn end_round(&mut self) {
        let a = self.team_score(Team::Fox, self.race_time);
        let b = self.team_score(Team::Falco, self.race_time);
        let mut anyone = false;
        for p in &mut self.players {
            if p.run.as_ref().is_some_and(Run::active) {
                p.played = true;
                anyone = true;
            }
        }
        let m = &mut self.teams;
        m.round_winner = match a.total_cmp(&b) {
            std::cmp::Ordering::Equal => None,
            std::cmp::Ordering::Greater => Some(Team::Fox),
            std::cmp::Ordering::Less => Some(Team::Falco),
        };
        match m.round_winner {
            Some(Team::Fox) => m.fox += 1,
            Some(Team::Falco) => m.falco += 1,
            None => {}
        }
        let won = m.fox.max(m.falco) >= POINTS_TO_WIN;
        if let Some(empty) = self.empty_team() {
            self.end_match(Some(empty.other()).filter(|t| self.team_size(*t) > 0));
        } else if won || !anyone || self.teams.round >= MAX_ROUNDS {
            let champion = match self.teams.fox.cmp(&self.teams.falco) {
                std::cmp::Ordering::Equal => None,
                std::cmp::Ordering::Greater => Some(Team::Fox),
                std::cmp::Ordering::Less => Some(Team::Falco),
            };
            self.end_match(champion);
        }
    }

    fn end_match(&mut self, champion: Option<Team>) {
        self.teams.over = true;
        self.teams.champion = champion;
        let Some(champion) = champion else { return };
        let mut ids = Vec::new();
        for p in &mut self.players {
            if p.team == Some(champion) && p.played {
                p.wins += 1;
                ids.push(p.id);
            }
        }
        for id in ids {
            self.update_stats(id, |s| s.team_wins += 1);
        }
    }

    pub fn chat_open(&self, now: Instant) -> bool {
        match self.phase {
            Phase::Lobby { start_at: None } => true,
            Phase::Lobby { start_at: Some(t) } => t.saturating_duration_since(now) > CHAT_LOCK,
            Phase::Racing { .. } | Phase::Briefing { .. } | Phase::Boss => false,
            Phase::Picking { until, .. } => until.saturating_duration_since(now) > CHAT_LOCK,
            Phase::Results { until } => {
                let left = until.saturating_duration_since(now);
                // Entre deux manches, la suivante démarre sans repasser par le salon.
                left <= RESULTS - CHAT_REOPEN && (!self.match_on() || left > CHAT_LOCK)
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
        self.chat.push_back(Message { color: p.color, name: p.name.clone(), text });
        if self.chat.len() > CHAT_HISTORY {
            self.chat.pop_front();
        }
        true
    }

    pub fn key(&mut self, id: u64, c: char, now: Instant) {
        if self.phase == Phase::Boss {
            if let Some(f) = &mut self.fight {
                f.key(id, c, now);
            }
            return self.tick(now);
        }
        let Phase::Racing { start, ends } = self.phase else { return };
        if now >= ends {
            return;
        }
        let text = self.text().as_bytes();
        let Some(p) = self.players.iter_mut().find(|p| p.id == id) else { return };
        let Some(run) = p.run.as_mut().filter(|r| r.finish.is_none()) else { return };

        if !run.keys.allow(now) {
            return;
        }

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
            let (wpm, accuracy) = (run.wpm(time), run.accuracy());
            let name = p.name.clone();
            let runs = || self.players.iter().filter_map(|p| p.run.as_ref());
            let first = runs().filter(|r| r.finish.is_some()).count() == 1;
            let won = first && self.mode == Mode::Solo && runs().count() > 1;
            if first {
                self.phase = Phase::Racing { start, ends: ends.min(now + AFTER_FIRST) };
            }
            if won {
                self.players.iter_mut().find(|p| p.id == id).unwrap().wins += 1;
            }
            self.update_stats(id, |s| {
                s.finished(wpm, accuracy);
                s.wins += won as u64;
            });
            self.record(name, wpm);
        }
        self.tick(now);
    }

    fn record(&mut self, name: String, wpm: u32) {
        match self.top.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            Some(e) => e.1 = e.1.max(wpm),
            None => self.top.push((name, wpm)),
        }
        self.top.sort_by_key(|e| std::cmp::Reverse(e.1));
        self.top.truncate(MAX_RECORDS);
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
                    Some(t) if now >= t => match self.next_mode() {
                        Mode::Solo => self.start(Mode::Solo, now),
                        Mode::Boss => {
                            self.mode = Mode::Boss;
                            self.solo = false;
                            self.phase = Phase::Briefing { until: now + BRIEFING };
                        }
                        Mode::Teams => {
                            self.clear_teams();
                            self.phase = Phase::Picking { until: now + PICK, last: false };
                        }
                    },
                    Some(_) => {}
                }
            }
            Phase::Picking { until, last } => {
                if self.ready_count() < 2 {
                    self.clear_teams();
                    self.phase = Phase::Lobby { start_at: None };
                } else if now >= until {
                    self.end_picking(last, now);
                } else if self.empty_team().is_none()
                    && self.players.iter().all(|p| !p.ready || p.team.is_some())
                {
                    self.phase = Phase::Picking { until: until.min(now + PICK_DONE), last };
                }
            }
            Phase::Briefing { until } => {
                if self.ready_count() == 0 {
                    self.phase = Phase::Lobby { start_at: None };
                } else if now >= until {
                    self.start_fight(now);
                }
            }
            Phase::Boss => {
                let over = self.fight.as_mut().is_none_or(|f| {
                    f.tick(now);
                    f.won.is_some()
                });
                if over {
                    self.end_fight(now);
                }
            }
            Phase::Racing { start, ends } => {
                let forfeit = self.mode == Mode::Teams && self.empty_team().is_some();
                let done = self
                    .players
                    .iter()
                    .filter_map(|p| p.run.as_ref())
                    .all(|r| r.finish.is_some());
                if done || forfeit || now >= ends {
                    self.race_time = now.min(ends) - start;
                    self.phase = Phase::Results { until: now + RESULTS };
                    if self.mode == Mode::Teams {
                        self.end_round();
                    }
                }
            }
            Phase::Results { until } => {
                if self.match_on()
                    && let Some(empty) = self.empty_team()
                {
                    // Un camp s'est vidé entre deux manches : forfait.
                    self.end_match(Some(empty.other()).filter(|t| self.team_size(*t) > 0));
                    self.phase = Phase::Results { until: now + RESULTS };
                } else if now >= until && self.match_on() {
                    self.start(Mode::Teams, now);
                } else if now >= until {
                    let fight = self.fight.take();
                    for p in &mut self.players {
                        if let Some(f) = fight.as_ref().and_then(|f| f.fighter(p.id)) {
                            p.ready = f.active();
                        }
                        // Aucune frappe pendant la course : joueur absent.
                        if let Some(r) = p.run.take() {
                            p.ready = r.active() || p.played;
                        }
                    }
                    self.clear_teams();
                    self.solo = false;
                    self.phase = Phase::Lobby { start_at: None };
                    self.tick(now);
                }
            }
        }
    }

    fn start(&mut self, mode: Mode, now: Instant) {
        if self.bag.is_empty() {
            self.bag = phrases::shuffled(Some(self.text));
        }
        self.text = self.bag.pop().unwrap();
        self.mode = mode;
        if mode == Mode::Teams {
            self.teams.round += 1;
            self.teams.round_winner = None;
        }
        for p in &mut self.players {
            let racing = match mode {
                Mode::Solo => p.ready,
                Mode::Teams => p.team.is_some(),
                Mode::Boss => false, // voir `start_fight`
            };
            p.run = racing.then_some(Run {
                pos: 0,
                errors: 0,
                wrong: false,
                finish: None,
                keys: Throttle::new(now),
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
        g.join(1, "ada", Stats::default(), t);
        g.join(2, "bob", Stats::default(), t);
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
        g.join(1, "ada", Stats::default(), t);
        assert_eq!(g.phase, Phase::Lobby { start_at: None });
        g.join(2, "bob", Stats::default(), t);
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + COUNTDOWN) });
        g.leave(2, t);
        assert_eq!(g.phase, Phase::Lobby { start_at: None });
    }

    #[test]
    fn solo_start() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", Stats::default(), t);
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
        g.join(3, "eve", Stats::default(), t);
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
        g.join(1, "a", Stats::default(), t);
        g.join(2, "b", Stats::default(), t);
        g.join(3, "c", Stats::default(), t + secs(9));
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + secs(14)) });
        for (i, s) in [13, 17, 21, 25].into_iter().enumerate() {
            g.join(4 + i as u64, "d", Stats::default(), t + secs(s));
        }
        assert_eq!(g.phase, Phase::Lobby { start_at: Some(t + MAX_COUNTDOWN) });
        g.tick(t + MAX_COUNTDOWN);
        assert_eq!(g.ranking().len(), 7);
    }

    #[test]
    fn everyone_ready_races() {
        let t = Instant::now();
        let mut g = Game::new();
        for id in 0..100 {
            g.join(id, "a", Stats::default(), t);
        }
        g.tick(t + MAX_COUNTDOWN);
        assert_eq!(g.ranking().len(), 100);
    }

    #[test]
    fn chat_closed_around_race_and_rate_limited() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", Stats::default(), t);
        g.join(2, "bob", Stats::default(), t);
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

    #[test]
    fn every_text_plays_once_before_repeating() {
        let t = Instant::now();
        let mut g = Game::new();
        let n = phrases::TEXTS.len();
        let mut seen = Vec::new();
        for _ in 0..2 * n {
            g.start(Mode::Solo, t);
            seen.push(g.text);
        }
        for round in seen.chunks(n) {
            assert_eq!(round.iter().collect::<std::collections::HashSet<_>>().len(), n);
        }
        assert_ne!(seen[n - 1], seen[n]);
    }

    #[test]
    fn chat_colors_are_distinct() {
        let t = Instant::now();
        let mut g = Game::new();
        for id in 0..64 {
            g.join(id, "a", Stats::default(), t);
        }
        let colors: std::collections::HashSet<_> =
            g.players[..CHAT_COLORS].iter().map(|p| p.color).collect();
        assert_eq!(colors.len(), CHAT_COLORS);
        assert!(colors.iter().all(|c| *c < CHAT_COLORS));
    }

    #[test]
    fn newcomer_avoids_color_of_someone_who_left() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "ada", Stats::default(), t);
        g.say(1, "salut", t);
        let old = g.player(1).unwrap().color;
        g.leave(1, t);
        g.join(2, "bob", Stats::default(), t);
        assert_ne!(g.player(2).unwrap().color, old);
    }

    /// n joueurs dont les `teams` premiers votent équipes, compte à rebours écoulé.
    fn voted(n: u64, teams: u64) -> (Game, Instant) {
        let t = Instant::now();
        let mut g = Game::new();
        for id in 1..=n {
            g.join(id, "p", Stats::default(), t);
            if id <= teams {
                g.cycle_vote(id, true);
            }
        }
        let t = t + COUNTDOWN;
        g.tick(t);
        (g, t)
    }

    /// Match à 2 contre 2 lancé : 1 et 2 Fox, 3 et 4 Falco.
    fn in_match() -> (Game, Instant) {
        let (mut g, t) = voted(4, 4);
        for id in 1..=4 {
            g.choose(id, if id <= 2 { Team::Fox } else { Team::Falco }, t);
        }
        let t = t + PICK_DONE;
        g.tick(t);
        assert!(g.match_on());
        (g, t)
    }

    /// `id` gagne la manche pour son camp, l'autre camp tape une touche fausse.
    fn round(g: &mut Game, id: u64, t: Instant) -> Instant {
        g.key(if id <= 2 { 3 } else { 1 }, '~', t + secs(1));
        let t = type_all(g, id, t) + AFTER_FIRST;
        g.tick(t);
        assert!(matches!(g.phase, Phase::Results { .. }));
        t
    }

    #[test]
    fn arrows_vote_in_lobby() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "p", Stats::default(), t);
        g.arrow(1, true, t);
        assert_eq!(g.votes(), [0, 1, 0]);
        g.arrow(1, false, t);
        assert_eq!(g.votes(), [1, 0, 0]);
        g.arrow(1, false, t);
        assert_eq!(g.votes(), [0, 0, 1]);
        assert_eq!(g.next_mode(), Mode::Boss);
        assert!(g.player(1).unwrap().team.is_none());
    }

    #[test]
    fn majority_picks_mode() {
        assert!(matches!(voted(4, 2).0.phase, Phase::Racing { .. }));
        assert!(matches!(voted(4, 3).0.phase, Phase::Picking { .. }));
    }

    #[test]
    fn players_choose_and_undecided_fill_smaller_side() {
        let (mut g, t) = voted(5, 5);
        for id in 1..=3 {
            g.choose(id, Team::Fox, t);
        }
        g.choose(3, Team::Falco, t);
        assert!(g.chat_open(t));
        assert!(!g.chat_open(t + PICK - secs(2)));
        g.tick(t + PICK);
        assert!(g.match_on());
        let team = |id| g.player(id).unwrap().team;
        assert_eq!((team(1), team(2), team(3)), (Some(Team::Fox), Some(Team::Fox), Some(Team::Falco)));
        assert!(g.team_size(Team::Fox).abs_diff(g.team_size(Team::Falco)) == 1);
    }

    #[test]
    fn one_sided_choice_extends_once_then_goes_individual() {
        let (mut g, t) = voted(2, 2);
        g.choose(1, Team::Falco, t);
        g.choose(2, Team::Falco, t);
        g.tick(t + PICK);
        assert!(matches!(g.phase, Phase::Picking { last: true, .. }));
        g.tick(t + PICK * 2);
        assert!(matches!(g.phase, Phase::Racing { .. }));
        assert_eq!(g.mode, Mode::Solo);
    }

    #[test]
    fn best_of_three_goes_to_decider() {
        let (mut g, mut t) = in_match();
        for (n, id) in [1, 3, 2].into_iter().enumerate() {
            assert_eq!(g.teams.round, n as u32 + 1);
            t = round(&mut g, id, t);
            if n < 2 {
                assert!(!g.teams.over);
                assert!(!g.chat_open(t + RESULTS - secs(2)));
                t += RESULTS;
                g.tick(t);
                assert!(matches!(g.phase, Phase::Racing { .. }));
            }
        }
        assert!(g.teams.over);
        assert_eq!((g.teams.fox, g.teams.falco, g.teams.champion), (2, 1, Some(Team::Fox)));
        let wins = |id| g.player(id).unwrap().stats.team_wins;
        assert_eq!((wins(1), wins(2), wins(3), wins(4)), (1, 1, 0, 0));
        g.tick(t + RESULTS);
        assert!(matches!(g.phase, Phase::Lobby { .. }));
        assert!(g.players.iter().all(|p| p.team.is_none()));
    }

    #[test]
    fn two_points_end_the_match() {
        let (mut g, t) = in_match();
        let t = round(&mut g, 3, t) + RESULTS;
        g.tick(t);
        round(&mut g, 4, t);
        assert!(g.teams.over);
        assert_eq!(g.teams.champion, Some(Team::Falco));
    }

    #[test]
    fn newcomer_picks_side_between_rounds_only() {
        let (mut g, t) = in_match();
        g.join(9, "eve", Stats::default(), t);
        g.choose(9, Team::Falco, t);
        assert!(g.player(9).unwrap().team.is_none());
        let t = round(&mut g, 1, t);
        g.choose(9, Team::Falco, t);
        g.choose(1, Team::Falco, t);
        assert_eq!(g.player(1).unwrap().team, Some(Team::Fox));
        g.tick(t + RESULTS);
        assert!(g.player(9).unwrap().run.is_some());
    }

    #[test]
    fn forfeit_and_abandoned_matches_end() {
        let (mut g, t) = in_match();
        g.key(3, '~', t + secs(1));
        g.leave(1, t + secs(2));
        g.leave(2, t + secs(2));
        assert_eq!((g.teams.over, g.teams.champion), (true, Some(Team::Falco)));
        assert_eq!(g.player(3).unwrap().stats.team_wins, 1);

        let (mut g, t) = in_match();
        let t = round(&mut g, 1, t);
        g.leave(3, t);
        g.leave(4, t);
        assert_eq!((g.teams.over, g.teams.champion), (true, Some(Team::Fox)));

        let (mut g, t) = in_match();
        g.tick(t + RACE_LIMIT);
        assert_eq!((g.teams.over, g.teams.champion), (true, None));
    }

    #[test]
    fn boss_fight_from_lobby_to_stats() {
        for win in [true, false] {
            let t = Instant::now();
            let mut g = Game::new();
            for (id, name) in [(1, "a"), (2, "b")] {
                g.join(id, name, Stats::default(), t);
                g.cycle_vote(id, false);
            }
            let mut t = t + COUNTDOWN;
            g.tick(t);
            assert!(matches!(g.phase, Phase::Briefing { .. }));
            assert!(!g.chat_open(t));
            t += BRIEFING;
            g.tick(t);
            assert_eq!(g.phase, Phase::Boss);
            g.join(3, "eve", Stats::default(), t);
            g.key(3, 'a', t);
            // 1 se bat, 2 est absent.
            g.key(1, '~', t + secs(1));
            if win {
                g.fight.as_mut().unwrap().hp = 0;
            }
            while g.phase == Phase::Boss {
                t += secs(1);
                g.tick(t);
            }
            let s = |id| g.player(id).unwrap().stats.clone();
            assert_eq!((s(1).boss_fights, s(1).boss_wins), (1, win as u64));
            assert_eq!((s(2).boss_fights, s(3).boss_fights), (0, 0));
            assert_eq!(s(1).races, 0);
            g.tick(t + RESULTS);
            assert!(matches!(g.phase, Phase::Lobby { .. }));
            assert!(g.fight.is_none());
            assert_eq!((g.player(1).unwrap().ready, g.player(2).unwrap().ready), (true, false));
        }
    }

    #[test]
    fn everyone_leaving_boss_returns_to_lobby() {
        let t = Instant::now();
        let mut g = Game::new();
        g.join(1, "a", Stats::default(), t);
        g.cycle_vote(1, false);
        g.press_enter(1, t);
        g.tick(t + SOLO_COUNTDOWN);
        g.tick(t + SOLO_COUNTDOWN + BRIEFING);
        assert_eq!(g.phase, Phase::Boss);
        g.leave(1, t + SOLO_COUNTDOWN + BRIEFING);
        g.tick(t + secs(60));
        assert_eq!(g.phase, Phase::Lobby { start_at: None });
    }
}
