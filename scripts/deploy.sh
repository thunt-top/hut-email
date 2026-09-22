#!/usr/bin/env bash
# Deploy a released image to production. Run by hand on the bridge machine,
# which has GitHub access and SSH access to production; production has neither.
#
# Usage: scripts/deploy.sh [tag]
#   tag defaults to the latest GitHub release.
#
# Requires: `gh` authenticated (gh auth login), and an SSH config alias
# for production set via PROD_HOST (see ~/.ssh/config).
set -euo pipefail

REPO="thunt-top/hut-email"
PROD_HOST="${PROD_HOST:-thunt-tx}"
PROD_DIR="${PROD_DIR:-/home/thunt/hut-email-prod}"

TAG="${1:-}"
if [ -z "$TAG" ]; then
  TAG=$(gh release view --repo "$REPO" --json tagName -q .tagName)
fi

echo "Deploying $TAG to $PROD_HOST:$PROD_DIR"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

gh release download "$TAG" --repo "$REPO" --pattern '*.tar.gz' --dir "$WORKDIR"
ARCHIVE=$(ls "$WORKDIR"/*.tar.gz)

ssh "$PROD_HOST" "mkdir -p '$PROD_DIR/config'"
rsync -avz "$ARCHIVE" "$PROD_HOST:$PROD_DIR/image.tar.gz"
rsync -avz compose.yaml "$PROD_HOST:$PROD_DIR/compose.yaml"
# Versioned, non-secret config ships with the deploy. config/secret.toml and
# config/email_map.toml are environment-specific: they are created by hand on
# the production host and never transit the bridge machine.
rsync -avz config/config.toml "$PROD_HOST:$PROD_DIR/config/config.toml"

ssh "$PROD_HOST" bash -s -- "$TAG" "$PROD_DIR" <<'EOF'
set -euo pipefail
TAG="$1"
DIR="$2"
cd "$DIR"
docker load -i image.tar.gz
docker tag "hut-email:$TAG" hut-email:latest
rm image.tar.gz
docker compose up -d
EOF

echo "Deployed $TAG."
