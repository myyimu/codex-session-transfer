#!/usr/bin/env bash
set -euo pipefail

npx tauri build --bundles app
mkdir -p release
ditto -c -k --sequesterRsrc --keepParent \
  "src-tauri/target/release/bundle/macos/Codex 会话迁移.app" \
  "release/Codex-Session-Transfer-0.1.6-aarch64.zip"
