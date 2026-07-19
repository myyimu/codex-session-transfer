#!/usr/bin/env bash
set -euo pipefail

npx tauri build --bundles app
version="$(node -p "require('./package.json').version")"
arch="$(uname -m)"
mkdir -p release
ditto -c -k --sequesterRsrc --keepParent \
  "src-tauri/target/release/bundle/macos/Codex 会话迁移.app" \
  "release/Codex-Session-Transfer-${version}-${arch}.zip"
