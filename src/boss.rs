//! Combat coopératif contre le boss. Aucune E/S : le temps est passé en paramètre.

use std::time::{Duration, Instant};

use crate::game::Throttle;
use crate::phrases;

pub const NAME: &str = "LE COLOSSE DE SUIE";
pub const MAX_HP: i32 = 100;
pub const FAULT_DAMAGE: i32 = 3;
pub const MISS_DAMAGE: i32 = 25;
const PARRY_DAMAGE: u32 = 10;
/// Mots parfaits d'affilée pour passer au multiplicateur suivant.
pub const COMBO_STEP: u32 = 5;
pub const MAX_MULT: u32 = 3;

/// Durée visée du combat pour un groupe à son niveau habituel.
const TARGET_SECS: f64 = 45.0;
/// Dégâts par seconde attendus, par mpm : lettres, combo moyen, temps de parade.
const DPS_PER_WPM: f64 = 0.06;

const FIRST_ATTACK: Duration = Duration::from_secs(7);
const ENRAGE_AT: Duration = Duration::from_secs(80);
const ENRAGE_EVERY: Duration = Duration::from_secs(5);
const ENRAGE_DAMAGE: i32 = 8;
const ENRAGE_STEP: i32 = 4;
/// Après un changement de consigne, les frappes déjà lancées ne comptent pas comme fautes.
const GRACE: Duration = Duration::from_millis(700);

const PARRY_WORDS: &[&str] =
    &["esquive", "pare", "bloque", "recule", "saute", "roule", "garde", "contre", "baisse", "fuis"];

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Event {
    Hit { damage: u32, mult: u32 },
    Fault,
    Parried,
    Missed,
    Enraged(i32),
    Down,
}

#[derive(Clone)]
pub struct Parry {
    pub word: &'static str,
    pub pos: usize,
    pub wrong: bool,
    pub until: Instant,
    pub window: Duration,
}

#[derive(Clone)]
pub struct Fighter {
    pub id: u64,
    pub name: String,
    pub hp: i32,
    pub combo: u32,
    pub pos: usize,
    pub wrong: bool,
    pub damage: u32,
    pub parries: u32,
    pub errors: u32,
    pub parry: Option<Parry>,
    pub event: Option<(Event, Instant)>,
    /// Part des PV du boss due à ce joueur ; nulle une fois retirée.
    share: f64,
    /// Rang du texte en cours dans `Fight::texts`.
    text: usize,
    /// Mot en cours sans faute.
    clean: bool,
    grace: Instant,
    keys: Throttle,
}

impl Fighter {
    pub fn alive(&self) -> bool {
        self.hp > 0
    }

    pub fn active(&self) -> bool {
        self.damage + self.parries + self.errors > 0
    }

    pub fn mult(&self) -> u32 {
        (1 + self.combo / COMBO_STEP).min(MAX_MULT)
    }

    fn hurt(&mut self, amount: i32, event: Event, now: Instant) {
        self.hp = (self.hp - amount).max(0);
        self.combo = 0;
        self.event = Some((event, now));
        if self.hp == 0 {
            self.parry = None;
            self.event = Some((Event::Down, now));
        }
    }
}

#[derive(Clone)]
pub struct Fight {
    pub hp: u32,
    pub max: u32,
    pub fighters: Vec<Fighter>,
    pub start: Instant,
    pub won: Option<bool>,
    pub time: Duration,
    texts: Vec<usize>,
    next_attack: Instant,
    next_enrage: Instant,
    enrage: i32,
}

impl Fight {
    /// `players` : identifiant, nom et mpm habituel.
    pub fn new(players: &[(u64, String, u32)], now: Instant) -> Self {
        let power: f64 = players.iter().map(|p| p.2.clamp(20, 130) as f64).sum();
        let max = (power * DPS_PER_WPM * TARGET_SECS).round().max(1.0) as u32;
        let fighters = players
            .iter()
            .map(|(id, name, level)| Fighter {
                id: *id,
                name: name.clone(),
                hp: MAX_HP,
                combo: 0,
                pos: 0,
                wrong: false,
                damage: 0,
                parries: 0,
                errors: 0,
                parry: None,
                event: None,
                share: (*level).clamp(20, 130) as f64 / power,
                text: 0,
                clean: true,
                grace: now,
                keys: Throttle::new(now),
            })
            .collect();
        Fight {
            hp: max,
            max,
            fighters,
            start: now,
            won: None,
            time: Duration::ZERO,
            texts: phrases::shuffled(None),
            next_attack: now + FIRST_ATTACK,
            next_enrage: now + ENRAGE_AT,
            enrage: ENRAGE_DAMAGE,
        }
    }

    pub fn fighter(&self, id: u64) -> Option<&Fighter> {
        self.fighters.iter().find(|f| f.id == id)
    }

    pub fn text(&self, f: &Fighter) -> &'static str {
        phrases::TEXTS[self.texts[f.text % self.texts.len()]]
    }

    /// 1 à 3 : plus le boss est blessé, plus il attaque.
    pub fn stage(&self) -> u32 {
        match self.hp as f64 / self.max as f64 {
            r if r > 0.6 => 1,
            r if r > 0.25 => 2,
            _ => 3,
        }
    }

    pub fn attacking(&self) -> bool {
        self.fighters.iter().any(|f| f.parry.is_some())
    }

    /// Temps avant l'enrage ; nul une fois enragé.
    pub fn enrage_in(&self, now: Instant) -> Duration {
        (self.start + ENRAGE_AT).saturating_duration_since(now)
    }

    /// Retire du boss la part de PV d'un joueur parti ou absent.
    fn drop_share(&mut self, id: u64) {
        let total: f64 = self.fighters.iter().map(|f| f.share).sum();
        let Some(f) = self.fighters.iter_mut().find(|f| f.id == id) else { return };
        let keep = 1.0 - std::mem::take(&mut f.share) / total.max(f64::EPSILON);
        // Dernier joueur : rien à réduire, le combat est perdu.
        if keep > 0.01 {
            self.hp = (self.hp as f64 * keep).ceil() as u32;
            self.max = (self.max as f64 * keep).ceil() as u32;
        }
    }

    pub fn leave(&mut self, id: u64) {
        self.drop_share(id);
        self.fighters.retain(|f| f.id != id);
    }

    pub fn key(&mut self, id: u64, c: char, now: Instant) {
        if self.won.is_some() {
            return;
        }
        let Some(i) = self.fighters.iter().position(|f| f.id == id && f.alive()) else { return };
        let text = self.text(&self.fighters[i]).as_bytes();
        let f = &mut self.fighters[i];
        if !f.keys.allow(now) {
            return;
        }

        if let Some(p) = &mut f.parry {
            if now >= p.until {
                return; // le prochain tick comptera le coup
            }
            if p.word.as_bytes()[p.pos] as char != c {
                p.wrong = now >= f.grace;
                return;
            }
            p.wrong = false;
            p.pos += 1;
            if p.pos == p.word.len() {
                f.parry = None;
                f.parries += 1;
                f.damage += PARRY_DAMAGE;
                f.grace = now + GRACE;
                f.event = Some((Event::Parried, now));
                self.hp = self.hp.saturating_sub(PARRY_DAMAGE);
            }
            return;
        }

        if text[f.pos] as char != c {
            if now >= f.grace {
                f.errors += 1;
                f.wrong = true;
                f.clean = false;
                f.hurt(FAULT_DAMAGE, Event::Fault, now);
            }
            return;
        }
        f.wrong = false;
        f.pos += 1;
        let done = f.pos == text.len();
        if c == ' ' || done {
            let end = if c == ' ' { f.pos - 1 } else { f.pos };
            let begin = text[..end].iter().rposition(|b| *b == b' ').map_or(0, |s| s + 1);
            let letters = (end - begin) as u32;
            let mult = f.mult();
            let damage = if f.clean { letters * mult } else { letters / 2 };
            if f.clean {
                f.combo += 1;
            }
            f.clean = true;
            f.damage += damage;
            f.event = Some((Event::Hit { damage, mult }, now));
            self.hp = self.hp.saturating_sub(damage);
        }
        if done {
            f.pos = 0;
            f.text += 1;
        }
    }

    /// Fait agir le boss ; renseigne `won` quand le combat est fini.
    pub fn tick(&mut self, now: Instant) {
        if self.won.is_some() {
            return;
        }
        if self.hp > 0 {
            self.act(now);
        }
        if self.hp == 0 {
            self.won = Some(true);
        } else if !self.fighters.iter().any(Fighter::alive) {
            self.won = Some(false);
        }
        if self.won.is_some() {
            self.time = now - self.start;
        }
    }

    fn act(&mut self, now: Instant) {
        for f in &mut self.fighters {
            if f.parry.as_ref().is_some_and(|p| now >= p.until) {
                f.parry = None;
                f.grace = now + GRACE;
                f.hurt(MISS_DAMAGE, Event::Missed, now);
            }
        }
        let absent = |f: &&Fighter| !f.alive() && !f.active() && f.share > 0.0;
        while let Some(id) = self.fighters.iter().find(absent).map(|f| f.id) {
            self.drop_share(id);
        }
        if now >= self.next_enrage {
            for f in self.fighters.iter_mut().filter(|f| f.alive()) {
                f.hurt(self.enrage, Event::Enraged(self.enrage), now);
            }
            self.enrage += ENRAGE_STEP;
            self.next_enrage = now + ENRAGE_EVERY;
        }
        if now >= self.next_attack {
            // Fenêtre de parade, puis répit avant l'attaque suivante.
            let (window, rest) = match self.stage() {
                1 => (4000, 6000..8000),
                2 => (4000, 4500..6500),
                _ => (3200, 3500..5000),
            };
            let window = Duration::from_millis(window);
            let word = PARRY_WORDS[rand::random_range(0..PARRY_WORDS.len())];
            for f in self.fighters.iter_mut().filter(|f| f.alive()) {
                f.parry = Some(Parry { word, pos: 0, wrong: false, until: now + window, window });
                f.grace = now + GRACE;
            }
            self.next_attack = now + window + Duration::from_millis(rand::random_range(rest));
        }
    }

    /// Du plus gros au plus petit total de dégâts.
    pub fn ranking(&self) -> Vec<&Fighter> {
        let mut v: Vec<&Fighter> = self.fighters.iter().collect();
        v.sort_by_key(|f| std::cmp::Reverse((f.damage, f.parries)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    fn fight(n: u64) -> (Fight, Instant) {
        let t = Instant::now();
        let players: Vec<_> = (1..=n).map(|i| (i, format!("p{i}"), 60)).collect();
        (Fight::new(&players, t), t)
    }

    /// Frappe correcte suivante de `id`, parade comprise.
    fn next_key(f: &Fight, id: u64) -> char {
        let p = f.fighter(id).unwrap();
        match &p.parry {
            Some(q) => q.word.as_bytes()[q.pos] as char,
            None => f.text(p).as_bytes()[p.pos] as char,
        }
    }

    #[test]
    fn clean_words_build_combo_and_faults_break_it() {
        let (mut f, mut t) = fight(1);
        f.next_attack = t + Duration::from_secs(99);
        while f.fighter(1).unwrap().combo < COMBO_STEP {
            t += ms(100);
            f.key(1, next_key(&f, 1), t);
        }
        assert_eq!(f.fighter(1).unwrap().mult(), 2);
        assert_eq!(f.max - f.hp, f.fighter(1).unwrap().damage);
        f.key(1, '~', t + ms(100));
        let p = f.fighter(1).unwrap();
        assert_eq!((p.combo, p.hp, p.errors), (0, MAX_HP - FAULT_DAMAGE, 1));
    }

    #[test]
    fn attack_parried_or_taken_with_grace() {
        let (mut f, t) = fight(2);
        let mut t = t + FIRST_ATTACK;
        f.tick(t);
        assert!(f.attacking());
        // Frappes déjà lancées au moment de l'attaque : ignorées.
        f.key(1, '~', t + ms(100));
        assert!(!f.fighter(1).unwrap().parry.as_ref().unwrap().wrong);
        while f.fighter(1).unwrap().parry.is_some() {
            t += ms(100);
            f.key(1, next_key(&f, 1), t);
        }
        f.key(1, '~', t + ms(200));
        assert_eq!((f.fighter(1).unwrap().hp, f.fighter(1).unwrap().parries), (MAX_HP, 1));
        f.tick(t + Duration::from_secs(4));
        assert_eq!(f.fighter(2).unwrap().hp, MAX_HP - MISS_DAMAGE);
        assert_eq!(f.fighter(1).unwrap().hp, MAX_HP);
    }

    #[test]
    fn idle_group_loses_and_enrage_ends_any_fight() {
        let (mut f, mut t) = fight(3);
        while f.won.is_none() {
            t += ms(50);
            f.tick(t);
        }
        assert_eq!(f.won, Some(false));

        // Pare tout mais ne frappe jamais : l'enrage finit par gagner.
        let (mut f, mut t) = fight(1);
        let start = t;
        while f.won.is_none() {
            t += ms(100);
            f.tick(t);
            if f.fighter(1).unwrap().parry.is_some() {
                f.key(1, next_key(&f, 1), t);
            }
            assert!(t - start < Duration::from_secs(200));
        }
        assert_eq!(f.won, Some(false));
    }

    #[test]
    fn steady_players_win_in_reasonable_time() {
        for n in [1, 2, 8] {
            let (mut f, mut t) = fight(n);
            // 60 mpm : une frappe toutes les 200 ms, une faute toutes les 40 frappes.
            let mut i = 0;
            while f.won.is_none() {
                t += ms(50);
                f.tick(t);
                i += 1;
                if i % 4 == 0 {
                    for id in 1..=n {
                        let c = if i % 160 == 0 { '~' } else { next_key(&f, id) };
                        f.key(id, c, t);
                    }
                }
            }
            assert_eq!(f.won, Some(true), "{n} joueurs");
            assert!((15..60).contains(&f.time.as_secs()), "{n} joueurs : {:?}", f.time);
        }
    }

    #[test]
    fn leaving_players_do_not_block_the_end() {
        let (mut f, t) = fight(2);
        f.leave(1);
        f.leave(2);
        f.tick(t);
        assert_eq!(f.won, Some(false));
    }

    #[test]
    fn leaver_takes_his_share_of_boss_hp() {
        let (mut f, _) = fight(4);
        let (hp, max) = (f.hp, f.max);
        f.leave(1);
        assert_eq!((f.hp, f.max), ((hp * 3).div_ceil(4), (max * 3).div_ceil(4)));
        f.leave(2);
        f.leave(3);
        let hp = f.hp;
        f.leave(4);
        assert_eq!(f.hp, hp);
    }

    #[test]
    fn absent_player_stops_counting_once_down() {
        let (mut f, mut t) = fight(2);
        let max = f.max;
        f.key(1, next_key(&f, 1), t);
        f.fighters[0].damage = 1;
        while f.fighter(2).unwrap().alive() {
            t += ms(100);
            f.tick(t);
            f.fighters[0].hp = MAX_HP;
        }
        f.tick(t + ms(100));
        assert!(f.max <= max / 2 + 1 && f.won.is_none());
    }
}
