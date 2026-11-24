mod game;
mod input;
mod phrases;
mod records;
mod session;
mod ui;

use std::collections::HashMap;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use russh::keys::ssh_key::LineEnding;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Auth, ChannelOpenHandle, Handler, Msg, Session};
use russh::{Channel, ChannelId, MethodKind, MethodSet, Pty};
use tokio::net::TcpListener;

use crate::game::Game;
use crate::session::Shared;

const MAX_CONNECTIONS: usize = 64;
const MAX_PER_IP: usize = 8;
const LOGIN_DEADLINE: Duration = Duration::from_secs(20);

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

struct Client {
    game: Shared,
    user: String,
    /// Passe à vrai à l'ouverture de la session (une seule par connexion).
    opened: Arc<AtomicBool>,
}

impl Handler for Client {
    type Error = anyhow::Error;

    // Serveur public : aucune identité requise, et aucun shell n'existe derrière.
    async fn auth_none(&mut self, user: &str) -> Result<Auth> {
        self.user = user.to_string();
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        channel: Channel<Msg>,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<()> {
        if self.opened.swap(true, Ordering::SeqCst) {
            return Ok(()); // `reply` abandonné = canal refusé
        }
        reply.accept().await;
        let (rx, tx) = channel.split();
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(session::run(self.game.clone(), id, self.user.clone(), rx, tx));
        Ok(())
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        _cols: u32,
        _rows: u32,
        _pw: u32,
        _ph: u32,
        _modes: &[(Pty, u32)],
        session: &mut Session,
    ) -> Result<()> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<()> {
        session.channel_success(channel)?;
        Ok(())
    }

    async fn exec_request(&mut self, channel: ChannelId, _: &[u8], session: &mut Session) -> Result<()> {
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, _: &str, session: &mut Session) -> Result<()> {
        session.channel_failure(channel)?;
        Ok(())
    }
}

/// La clé d'hôte doit survivre aux redémarrages, sinon les clients SSH alertent.
fn host_key(path: &Path) -> Result<PrivateKey> {
    if path.exists() {
        return PrivateKey::read_openssh_file(path).context("lecture de la clé d'hôte");
    }
    let key = PrivateKey::random(&mut russh::keys::key::safe_rng(), Algorithm::Ed25519)?;
    key.write_openssh_file(path, LineEnding::LF) // créé en 0600
        .with_context(|| format!("écriture de {}", path.display()))?;
    Ok(key)
}

/// Place réservée pour une IP, rendue à la déconnexion.
struct Slot {
    ips: Arc<Mutex<HashMap<IpAddr, usize>>>,
    ip: IpAddr,
}

impl Slot {
    fn take(ips: &Arc<Mutex<HashMap<IpAddr, usize>>>, ip: IpAddr) -> Option<Slot> {
        let mut map = ips.lock().unwrap();
        if map.values().sum::<usize>() >= MAX_CONNECTIONS {
            return None;
        }
        if map.get(&ip).copied().unwrap_or(0) >= MAX_PER_IP {
            return None;
        }
        *map.entry(ip).or_default() += 1;
        Some(Slot { ips: ips.clone(), ip })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut map = self.ips.lock().unwrap();
        if let Some(n) = map.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                map.remove(&self.ip);
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    // Sortie vers des clients distants : NO_COLOR côté serveur n'a pas de sens.
    crossterm::style::force_color_output(true);
    let listen = std::env::var("LISTEN").unwrap_or_else(|_| "0.0.0.0:2222".into());
    let key_path = std::env::var("HOST_KEY").unwrap_or_else(|_| "host_key".into());

    let mut methods = MethodSet::empty();
    methods.push(MethodKind::None);
    let config = Arc::new(server::Config {
        methods,
        keys: vec![host_key(Path::new(&key_path))?],
        auth_rejection_time: Duration::from_secs(1),
        auth_rejection_time_initial: Some(Duration::ZERO),
        max_auth_attempts: 3,
        keepalive_interval: Some(Duration::from_secs(30)),
        keepalive_max: 3,
        nodelay: true,
        ..Default::default()
    });

    let records_path =
        PathBuf::from(std::env::var("RECORDS").unwrap_or_else(|_| "records.txt".into()));
    let mut state = Game::new();
    // Illisible : on refuse de démarrer plutôt que d'écraser les records.
    state.top = records::load(&records_path).context("lecture des records")?;
    let mut saved = state.top.clone();
    let game: Shared = Arc::new(Mutex::new(state));
    let clock = game.clone();
    tokio::spawn(async move {
        let mut t = tokio::time::interval(Duration::from_millis(50));
        loop {
            t.tick().await;
            let top = {
                let mut g = clock.lock().unwrap_or_else(|e| e.into_inner());
                g.tick(Instant::now());
                g.top.clone()
            };
            if top != saved {
                if let Err(e) = records::save(&records_path, &top) {
                    eprintln!("sauvegarde des records : {e}");
                }
                saved = top;
            }
        }
    });

    let ips = Arc::new(Mutex::new(HashMap::new()));
    let listener = TcpListener::bind(&listen).await?;
    println!("ssh-typing écoute sur {listen}");
    loop {
        let (stream, peer) = match listener.accept().await {
            Ok(c) => c,
            Err(e) => {
                eprintln!("accept: {e}");
                tokio::time::sleep(Duration::from_millis(200)).await;
                continue;
            }
        };
        let Some(slot) = Slot::take(&ips, peer.ip()) else { continue };
        let _ = stream.set_nodelay(true);
        let opened = Arc::new(AtomicBool::new(false));
        let client = Client { game: game.clone(), user: String::new(), opened: opened.clone() };
        let config = config.clone();
        tokio::spawn(async move {
            let _slot = slot;
            let Ok(Ok(mut running)) =
                tokio::time::timeout(LOGIN_DEADLINE, server::run_stream(config, stream, client)).await
            else {
                return;
            };
            tokio::select! {
                _ = &mut running => return,
                _ = tokio::time::sleep(LOGIN_DEADLINE) => {}
            }
            if !opened.load(Ordering::SeqCst) {
                let _ = running
                    .handle()
                    .disconnect(russh::Disconnect::ByApplication, String::new(), String::new())
                    .await;
            }
            let _ = running.await;
        });
    }
}
