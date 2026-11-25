//! État partagé : la partie en cours et les comptes.

use std::time::Instant;

use crate::accounts::{Accounts, Stats};
use crate::game::{Game, clean_name};

pub struct World {
    pub game: Game,
    pub accounts: Accounts,
}

impl World {
    /// Reporte dans les comptes les stats modifiées par la partie.
    pub fn sync(&mut self) {
        for (name, stats) in self.game.outbox.drain(..) {
            self.accounts.update(&name, stats);
        }
    }

    pub fn tick(&mut self, now: Instant) {
        self.game.tick(now);
        self.sync();
    }

    /// Entre dans la partie avec le compte de ce pseudo.
    pub fn join(&mut self, id: u64, wanted: &str, now: Instant) -> Result<(), &'static str> {
        self.sync();
        let name = clean_name(wanted);
        if name.is_empty() {
            return Err("choisis un pseudo");
        }
        if self.game.players.iter().any(|p| p.name.eq_ignore_ascii_case(&name)) {
            return Err("ce pseudo est déjà connecté");
        }
        let (name, mut stats) = match self.accounts.by_name(&name) {
            Some(a) => (a.name.clone(), a.stats.clone()),
            None => (name, Stats::default()),
        };
        // Reprend le record d'avant les comptes.
        if let Some((_, wpm)) = self.game.top.iter().find(|(n, _)| n.eq_ignore_ascii_case(&name)) {
            stats.best = stats.best.max(*wpm);
        }
        self.game.join(id, &name, stats, now);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{COUNTDOWN, RACE_LIMIT, RESULTS};
    use std::time::Duration;

    #[test]
    fn same_name_gets_same_stats_and_record() {
        let t0 = Instant::now();
        let mut game = Game::new();
        game.top = vec![("zazic".into(), 110), ("lenny".into(), 90)];
        let mut w = World { game, accounts: Accounts::default() };
        w.join(1, "Lenny", t0).unwrap();
        w.join(2, "bob", t0).unwrap();
        assert!(w.join(3, "BOB", t0).is_err());
        assert!(w.join(3, "", t0).is_err());
        assert_eq!(w.game.player(1).unwrap().stats.best, 90);

        let mut t = t0 + COUNTDOWN;
        w.tick(t);
        assert!(w.accounts.by_name("bob").is_none());
        for c in w.game.text().chars() {
            t += Duration::from_millis(100);
            w.game.key(2, c, t);
            w.game.key(1, c, t);
        }
        w.tick(t + RACE_LIMIT);
        w.tick(t + RACE_LIMIT + RESULTS);
        assert!(w.accounts.dirty);
        assert!(w.game.top.iter().any(|(n, _)| n == "zazic"));
        assert_eq!(w.game.top.iter().filter(|(n, _)| n.eq_ignore_ascii_case("lenny")).count(), 1);

        w.game.leave(1, t);
        w.join(4, "LENNY", t).unwrap();
        let p = w.game.player(4).unwrap();
        assert_eq!((p.name.as_str(), p.stats.races, p.stats.accuracy()), ("Lenny", 1, Some(100)));

        // Relu depuis le disque ; un ancien champ « key » est ignoré.
        let path = std::env::temp_dir().join(format!("ssh-typing-acc-{}", std::process::id()));
        let json = w.accounts.to_json().replacen("\"name\"", "\"key\": \"x\", \"name\"", 1);
        std::fs::write(&path, json).unwrap();
        assert_eq!(Accounts::load(&path).unwrap().by_name("lenny").unwrap().stats.races, 1);
        std::fs::remove_file(path).unwrap();
    }
}
