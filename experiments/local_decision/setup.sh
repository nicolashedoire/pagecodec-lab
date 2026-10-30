#!/bin/sh
# Download only; does not enable a service or alter system policy.
set -eu
HERE=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
mkdir -p "$HERE/tmp" "$HERE/cache/pip" "$HERE/cache/huggingface" "$HERE/models"
export TMPDIR="$HERE/tmp"
export PIP_CACHE_DIR="$HERE/cache/pip"
export PIP_DISABLE_PIP_VERSION_CHECK=1
export HF_HOME="$HERE/cache/huggingface"
export HF_HUB_DISABLE_TELEMETRY=1
export HF_HUB_DISABLE_IMPLICIT_TOKEN=1
python3 -m venv "$HERE/.venv"
"$HERE/.venv/bin/python" -m pip install --only-binary=:all: -r "$HERE/requirements-lock.txt"
"$HERE/.venv/bin/python" - "$HERE/models/laya-multilingual-mlx" <<'PY'
import sys
from huggingface_hub import snapshot_download
snapshot_download(repo_id='aac6fef/laya-multilingual-mlx',
                  revision='f2b4faf51023039425946074e2cf1361d2db11d5',
                  local_dir=sys.argv[1], token=False)
PY
