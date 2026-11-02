# Storage authority and process slots

This note describes a proposed, unvalidated change; it is not part of the integrated kernel. It does not yet
introduce child creation, delegated privileges, or a general security sandbox.

The existing independent-process service ties tree access to a scheduler slot:
slot 0 accesses `/a`, slot 1 `/b`, and slot 2 `/c`. The flat catalog uses the
corresponding `a-`, `b-`, and `c-` prefixes. This is adequate only while the
controller assigns those identities deliberately. If a process creates a
child and the kernel simply selects an unused slot, the child could acquire a
different namespace from its parent. A slot identifies an execution location,
not an authority that should be delegated.

The proposal introduces immutable storage policy bound to a process incarnation.
It records a principal and separate tree-read, tree-write, flat-read, and
flat-write rights. Existing externally admitted processes receive the same
principal and rights they had before: their initial principal equals their
slot. A service resolves policy from the kernel-owned `(slot, incarnation)`
after validating a live, popped request; it never accepts a principal or rights
from the process. Authority is installed before the process becomes ready and
removed on reclaim. Historical pair-mode authorization keeps its prior mapping.
The service checks the policy for directory reads, mutations, staged writes
(including commit), tree reads, and the flat catalog.

No child policy or inheritance mechanism is implemented yet. A future process
spawn must derive an explicitly bounded subset of the parent's rights, carry the
parent's intended principal independently of the child's slot, and publish the
child only after its policy is in place. Avoiding escalation also requires an
audit of every other service, not only file access.

This draft does **not** constrain document bindings, direct IPC, network grants,
graphics surfaces or input focus, or the tree-generation discovery SVC. These
features continue to follow their existing rules. Surface placement and input
routing currently depend on slot; document/IPC/network grants have their own
incarnation checks. The MCP control endpoint is unauthenticated. No claim of
full process security follows from this storage-only policy.

A local draft has been compiled and statically reviewed, but is not included in
the integrated kernel. Virtual-machine scenarios still need to verify its
behavior, including stale incarnations, reclaim and readmission, ordinary
legacy operations, and denial cases. Those tests do
not prove cross-service isolation or physical-hardware behavior.
