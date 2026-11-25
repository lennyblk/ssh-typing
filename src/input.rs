//! Décodage du flux clavier. Conserve son état entre deux paquets SSH.

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Quit,
    Tab,
    Profile,
    Restart,
    Left,
    Right,
}

#[derive(Default)]
enum State {
    #[default]
    Ground,
    Esc,
    Ss3,
    Csi(Vec<u8>),
}

#[derive(Default)]
pub struct Parser {
    state: State,
    pasting: bool,
    /// Caractère UTF-8 en cours de réception.
    utf8: Vec<u8>,
}

fn arrow(b: u8) -> Option<Key> {
    match b {
        b'C' => Some(Key::Right),
        b'D' => Some(Key::Left),
        _ => None,
    }
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        let mut keys = Vec::new();
        for &b in bytes {
            match &mut self.state {
                State::Ground => {
                    if b >= 0x80 {
                        if b >= 0xc0 {
                            self.utf8.clear();
                        }
                        self.utf8.push(b);
                        if let Ok(s) = std::str::from_utf8(&self.utf8) {
                            if !self.pasting {
                                keys.extend(s.chars().map(Key::Char));
                            }
                            self.utf8.clear();
                        } else if self.utf8.len() >= 4 {
                            self.utf8.clear();
                        }
                        continue;
                    }
                    self.utf8.clear();
                    let key = match b {
                        0x1b => {
                            self.state = State::Esc;
                            continue;
                        }
                        3 | 4 => {
                            keys.push(Key::Quit);
                            continue;
                        }
                        b'\r' => Key::Enter,
                        b'\t' => Key::Tab,
                        0x12 => Key::Restart, // Ctrl+R
                        0x10 => Key::Profile, // Ctrl+P
                        8 | 0x7f => Key::Backspace,
                        0x20..=0x7e => Key::Char(b as char),
                        _ => continue,
                    };
                    // Le contenu collé ne compte pas comme de la frappe.
                    if !self.pasting {
                        keys.push(key);
                    }
                }
                State::Esc => {
                    self.state = match b {
                        b'[' => State::Csi(Vec::new()),
                        b'O' => State::Ss3,
                        0x1b => State::Esc,
                        3 | 4 => {
                            keys.push(Key::Quit);
                            State::Ground
                        }
                        _ => State::Ground,
                    }
                }
                State::Ss3 => {
                    keys.extend(arrow(b));
                    self.state = State::Ground;
                }
                State::Csi(params) => {
                    if (0x40..=0x7e).contains(&b) {
                        match (params.as_slice(), b) {
                            (b"200", b'~') => self.pasting = true,
                            (b"201", b'~') => self.pasting = false,
                            (b"", _) if !self.pasting => keys.extend(arrow(b)),
                            _ => {}
                        }
                        self.state = State::Ground;
                    } else if params.len() < 16 {
                        params.push(b);
                    } else {
                        self.state = State::Ground;
                    }
                }
            }
        }
        keys
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequences_and_split_packets() {
        let mut p = Parser::default();
        assert_eq!(p.feed(b"a\x1b["), vec![Key::Char('a')]);
        assert_eq!(p.feed(b"Ab\r"), vec![Key::Char('b'), Key::Enter]);
        assert_eq!(p.feed(b"\x1b[200~paste\x1b[201~c"), vec![Key::Char('c')]);
        assert_eq!(p.feed(b"\x1b[D\x1bOC\x1b[A"), vec![Key::Left, Key::Right]);
        assert_eq!(p.feed(b"\xc3"), vec![]);
        assert_eq!(p.feed(b"\xa9\xff\xff\xff\xff\xffz"), vec![Key::Char('é'), Key::Char('z')]);
        assert_eq!(p.feed(b"\x1b\x03"), vec![Key::Quit]);
    }
}
