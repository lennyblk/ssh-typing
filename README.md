# SSH Typing

Course de frappe multijoueur jouable dans n'importe quel terminal, via SSH.

```sh
cargo run --release              # serveur sur le port 2222
ssh -p 2222 localhost            # joueur
docker compose up -d --build     # production
cargo test
```

- Dès que deux joueurs sont prêts : départ dans 10 s. Seul : Entrée.
- 8 coureurs max. Rejoindre pendant le compte à rebours l'allonge si besoin (5 s mini, 30 s au total).
- Au-delà de 8 : file d'attente, prioritaire à la course suivante.
- Une mauvaise touche bloque jusqu'à la bonne. Le collage est ignoré.
- Arriver pendant une course : spectateur, puis engagé dans la suivante.
- Variables : `LISTEN` (`0.0.0.0:2222`), `HOST_KEY` (`host_key`, à conserver).
