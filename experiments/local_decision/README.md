# Local decision-model demonstration

This isolated demonstration runs the independent **Laya multilingual** model
using [Laya-MLX](https://github.com/mizorewww/laya-mlx). It is not TypeSafe's
Jev, a reproduction of Jev's weights, or a demonstrated equivalent. No OS
settings or kernel policy are changed. No autonomous service is installed.

On an Apple M4 Pro running macOS 15.1.1, the model ran locally with an entirely
fictional workload. In two executions of one scenario, it recommended more
compression even though the scenario says memory is sufficient and the CPU is
busy. The expected answer for this deliberately simple case was to favor the
foreground workload. One example cannot establish general accuracy or
calibration, but the recommendation demonstrates why this separate model must
not be assumed to share Jev's behavior or be connected directly to OS control.
The complete [first result](../../results/local-decision-2026-10-30/demo-result-python-offline.json)
and [sandboxed result](../../results/local-decision-2026-10-30/demo-result.json)
contain the exact prompt and outputs.

The first execution took 1.04 seconds to load and 1.22 seconds for its first
inference, with peak process RSS of 803 MiB. The second took 0.88 seconds and
0.062 seconds respectively, with 920 MiB peak RSS. These are two single
observations on a shared machine, potentially with different warm cache and
shader state; they do not measure steady-state performance or sandbox overhead.

The second run was launched under a macOS sandbox denying network access; an
independent socket probe was denied. The script also sets the model hub's
offline mode and disables Python socket connections. It receives a local
checkpoint and synthetic text, without reading machine telemetry or sending
data to a remote model. No Jev inference was performed.

## Reproduce on Apple Silicon

Requirements: macOS 14 or newer, an ARM64 Python 3.12 installation, and enough
disk for the isolated environment and model. Preparation downloads dependencies
and the checkpoint into this directory. Afterwards the demo runs offline.

```sh
cd experiments/local_decision
./setup.sh
./run-offline.sh
```

Generated environments, weights, caches, temporary files and new results are
ignored by Git. Published results are kept separately and are not overwritten.
The setup pins dependency versions and the checkpoint revision; dependency
wheel hashes are not independently pinned or audited. The demo verifies model
file sizes and hashes against the downloaded publisher manifest. This shows
consistency with that manifest, not an independent security audit.

The checkpoint is [aac6fef/laya-multilingual-mlx](https://huggingface.co/aac6fef/laya-multilingual-mlx)
at revision `f2b4faf51023039425946074e2cf1361d2db11d5` (Apache-2.0); its model
weights occupy 643,835,426 bytes. Runtime versions are Laya-MLX 0.6.0 and MLX
0.32.4. The public code and weights are not included in this repository.
