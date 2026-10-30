#!/bin/sh
set -eu
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
export HF_HOME="$HERE/cache/huggingface"
export HF_HUB_OFFLINE=1
export HF_HUB_DISABLE_TELEMETRY=1
export HF_HUB_DISABLE_IMPLICIT_TOKEN=1
export LOCAL_DECISION_OS_SANDBOX=1
export TMPDIR="$HERE/tmp"
exec /usr/bin/sandbox-exec -p '(version 1) (allow default) (deny network*)' "$HERE/.venv/bin/python" "$HERE/demo.py"
