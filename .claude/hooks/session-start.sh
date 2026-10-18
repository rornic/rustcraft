#!/bin/bash
# Sets up a Claude Code cloud session so the game builds, lints and tests.
set -euo pipefail

if [ "${CLAUDE_CODE_REMOTE:-}" != "true" ]; then
  exit 0
fi

# Bevy's audio and input link against ALSA and udev.
if ! pkg-config --exists alsa libudev 2>/dev/null; then
  apt-get update -qq
  apt-get install -y -qq pkg-config libasound2-dev libudev-dev
fi

# Bevy needs a newer rustc than the image may carry; stay on the latest stable.
rustup update stable --no-self-update
rustup component add clippy rustfmt --toolchain stable

cd "$CLAUDE_PROJECT_DIR"
cargo build --all-targets
