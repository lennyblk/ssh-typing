//! Décodage du flux clavier. Conserve son état entre deux paquets SSH.

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Key {
    Char(char),
    Enter,
    Backspace,
    Quit,
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
}

impl Parser {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Key> {
        let mut keys = Vec::new();
        for &b in bytes {
            match &mut self.state {
                State::Ground => {
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
                State::Ss3 => self.state = State::Ground,
                State::Csi(params) => {
                    if (0x40..=0x7e).contains(&b) {
                        match (params.as_slice(), b) {
                            (b"200", b'~') => self.pasting = true,
                            (b"201", b'~') => self.pasting = false,
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
        assert_eq!(p.feed(b"\x1b\x03"), vec![Key::Quit]);
    }
}
