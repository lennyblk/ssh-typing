#!/usr/bin/env bash
# Lancé sur le serveur par la CI (commande forcée de la clé de déploiement).
set -euo pipefail

# Dans une fonction : bash lit tout avant que git pull ne remplace ce fichier.
main() {
    cd "$(dirname "$(readlink -f "$0")")"
    git pull --ff-only origin main
    # Si le build échoue, l'ancienne version continue de tourner.
    docker compose build
    docker compose up -d
    sleep 5
    local state
    state=$(docker inspect -f '{{.State.Running}} {{.RestartCount}}' ssh-typing)
    if [ "$state" != "true 0" ]; then
        echo "ssh-typing ne tourne pas correctement ($state)" >&2
        docker compose logs --tail 30 >&2
        exit 1
    fi
    docker image prune -f --filter label=app=ssh-typing
    echo "déployé : $(git log -1 --format='%h %s')"
    exit 0
}

main
