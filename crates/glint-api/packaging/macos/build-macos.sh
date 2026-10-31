#!/bin/bash
# Keep the existing API release/CI entry point compatible.
set -euo pipefail
repo="$(cd "$(dirname "$0")/../../../.." && pwd)"
exec bash "$repo/packaging/macos/build-macos.sh" api "$@"
