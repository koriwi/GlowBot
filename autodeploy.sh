#!/bin/sh
set -e

before=$(git rev-parse HEAD)
git pull
after=$(git rev-parse HEAD)

if [ "$before" != "$after" ]; then
   cd /opt/stacks/sandbox docker compose up -d --build glowbot
fi
