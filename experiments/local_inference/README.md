# Tiny local integer inference

This Rust `no_std` experiment evaluates a **hand-written, synthetic neural-network-shaped calculation** locally in a QEMU guest. It has three bounded integer inputs, a dense four-unit hidden layer with ReLU, and two linear output scores. The fixed coefficients are invented for the demonstration; they are not trained weights, Jev, Laya, a policy or a validated model. Inputs are fictional numbers labelled pressure, CPU load and I/O activity; the guest does not observe a real system. Outputs are arbitrary signed scores, not probabilities, confidence levels or recommendations. Nothing in this experiment controls the machine.

`Input::new` rejects each component outside `0..=100`. For accepted inputs the maximum hidden activations are conservatively `[350, 420, 400, 330]`; the two score ranges are bounded by `[-740, 1110]` and `[-370, 1530]`. The fixed weights and these bounds keep intermediate and final calculations within `i32`. The library uses no allocation, floating point, network or hardware-specific acceleration. It does not establish that a model is useful, calibrated, safe, fast or energy-efficient.

The QEMU guest checks four exact, hand-calculated cases, repeats them to check deterministic output, rejects negative and over-limit values in each input, and checks score/hidden bounds at the eight input corners. The analytical bounds cover the permitted input range; the eight corner tests do not exhaustively test every input. The [recorded synthetic output](evidence/local-inference-evidence.json) contains no system data.

Integrating an actual model such as Jev or Laya would require a suitable available implementation and weights, compatible license, runtime and target support, memory capacity and explicit evaluation on representative system data. This tiny network is neither of those models. Latency, CPU use, power, model quality and practical integration costs have **not** been measured here.

## Reproduce

With Rust target `aarch64-unknown-none-softfloat`, QEMU AArch64 and Python 3:

```sh
cargo build --release --locked --offline --features qemu-demo
python3 run_demo.py --qemu /path/to/qemu-system-aarch64 \
  --guest target/aarch64-unknown-none-softfloat/release/local-inference-demo \
  --output-dir /path/to/output
```

The host only launches the one-CPU QEMU TCG guest and collects serial output. The guest has no network, disks or host-directory sharing. The runner checks the guest hash before and after the run and records QEMU version; do not rebuild the image while it runs. The exact fixtures demonstrate arithmetic behavior, not model quality or a production inference framework.
