//! Comptes : un pseudo = un compte, créé à la première course terminée.
//! Aucune preuve d'identité : c'est un jeu entre amis.

use std::path::Path;
use std::{fs, io};

use serde::{Deserialize, Serialize};

const HISTORY: usize = 20;
const MAX_ACCOUNTS: usize = 5000;

#[derive(Clone, Default, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Courses terminées.
    pub races: u64,
    pub wins: u64,
    pub team_wins: u64,
    pub boss_fights: u64,
    pub boss_wins: u64,
    /// Victoire la plus rapide contre le boss, en secondes ; 0 si aucune.
    pub boss_best: u32,
    pub best: u32,
    total_wpm: u64,
    total_accuracy: u64,
    /// Derniers mpm, du plus ancien au plus récent.
    pub history: Vec<u32>,
}

impl Stats {
    pub fn finished(&mut self, wpm: u32, accuracy: u32) {
        self.races += 1;
        self.best = self.best.max(wpm);
        self.total_wpm += wpm as u64;
        self.total_accuracy += accuracy as u64;
        self.history.push(wpm);
        if self.history.len() > HISTORY {
            self.history.remove(0);
        }
    }

    pub fn average(&self) -> Option<u32> {
        self.total_wpm.checked_div(self.races).map(|v| v as u32)
    }

    pub fn accuracy(&self) -> Option<u32> {
        self.total_accuracy.checked_div(self.races).map(|v| v as u32)
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    #[serde(default)]
    pub stats: Stats,
}

#[derive(Default)]
pub struct Accounts {
    list: Vec<Account>,
    pub dirty: bool,
}

impl Accounts {
    pub fn load(path: &Path) -> io::Result<Self> {
        let list = match fs::read(path) {
            Ok(data) => serde_json::from_slice(&data)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e),
        };
        Ok(Accounts { list, ..Default::default() })
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(&self.list).expect("comptes sérialisables")
    }

    pub fn by_name(&self, name: &str) -> Option<&Account> {
        self.list.iter().find(|a| a.name.eq_ignore_ascii_case(name))
    }

    pub fn update(&mut self, name: &str, stats: Stats) {
        let full = self.list.len() >= MAX_ACCOUNTS;
        match self.list.iter_mut().find(|a| a.name.eq_ignore_ascii_case(name)) {
            Some(a) => a.stats = stats,
            None if !full => {
                self.list.push(Account { name: name.into(), stats });
            }
            None => return,
        }
        self.dirty = true;
    }
}
