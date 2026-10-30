# Synthetic inference through the guest MCP service

[`decision_vm`](../experiments/decision_vm/README.md) now exposes the standalone [`local_inference`](../experiments/local_inference/README.md) arithmetic fixture as a synthetic MCP tool in a QEMU guest derived from [`concurrent_isolation`](../experiments/concurrent_isolation/README.md). Its [modern](../experiments/decision_vm/evidence/decision-vm-evidence.json) and [legacy](../experiments/decision_vm/evidence/decision-vm-legacy-evidence.json) traces cover selected requests alongside the graphical and EL0 fixtures. This does not make a system decision or establish an isolated model service.

## Implemented synthetic contract

The `infer_synthetic` tool accepts an object with exactly three required integer fields: `pressure`, `cpu_load` and `io_activity`, each from 0 through 100. These are **caller-provided synthetic numbers**, not authenticated measurements of real system state. The guest checks JSON types and ranges before converting to `i16` and using `Input::new`. The line protocol has a 512-byte request buffer and an overall message limit.

The response reports two signed integer scores and a fixture/coefficient version. These scores are raw results of a tiny invented calculation: **they are not probabilities, calibrated confidence, trained-model output, recommended actions or evidence of improved performance**. In particular, the fixture is neither Jev nor Laya. No model weights are downloaded and no system decision is made as a result of invoking this tool.

The guest parses JSON into `serde_json::Value`; duplicate object member names are not separately rejected and the last value is used. A strict duplicate-key rule needs a suitable parser or deserializer and its own tests. Synthetic fixtures may be retained in reproducible test evidence; routine logs should avoid exposing private inputs.

The recorded runs compare four exact result vectors, reject selected invalid inputs and check that pointer, focus, document and framebuffer state remain unchanged in the tested flows. Negative and high values are tested for each field, while other wrong-type cases mainly target `pressure`; rejection coverage is not exhaustive. The runs confirm EL0 progress at service boundaries, not execution inside the inference call. No latency is measured.

## Execution and boundaries

The arithmetic itself has fixed dimensions and analytically bounded integer values for accepted inputs. This does not establish a wall-clock deadline for the complete MCP request: parsing, serialization, UART polling, voluntary service yields and QEMU scheduling have their own costs. Measure end-to-end behavior if a latency claim is needed, including errors and competing work; do not infer a real-time guarantee from the small number of multiply-adds.

The existing service executes in EL1 and its MCP endpoint and actor labels are not authenticated. Adding a function call does not put inference in an EL0 process, authorize the caller or create a security boundary. The current bump allocator is finite and non-reclaiming, and the guest caps messages per boot. Tests adding inference requests must count initialization, notifications, retries and status checks against that cap. The EL0 test loops must continue to run only at the service's explicit yield boundaries; inference is not safe to call from an IRQ handler merely because the arithmetic is small.

## Further VM verification

1. List the tool and verify its published schema, then compare fixed synthetic input/output vectors with the standalone arithmetic fixture, including repeat calls and boundary inputs.
2. Exercise every field with missing, extra, wrong-type, negative, greater-than-100 and excessively large integer values; test malformed and oversized lines. If duplicate-key rejection is added, test it independently. Confirm correct error responses and unchanged state.
3. Check that inference leaves pointer, interaction and document snapshots unchanged, including after rejected requests. It must not change VM or host policy.
4. Record both EL0 task progress and the service's expected active root around inference requests, without claiming that EL0 executes in the middle of an individual handler. Run modern and legacy VirtIO scenarios if integrated with the graphical guest.
5. Use guest hashes, QEMU version and synthetic request/response evidence. Count all messages, bound retries and exercise resource exhaustion or failure explicitly; record which cases were actually observed.

Passing those tests would demonstrate a synthetic MCP calculation in one QEMU configuration. It would not establish trained-model quality, probability calibration, authenticated access, general process or DMA isolation, real-device safety, native Apple support or a performance improvement.
