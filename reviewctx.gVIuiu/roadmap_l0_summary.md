L0 target summary from ai-gateway-platform-deep-governance.dsl:
- L0 is the stateless invocation kernel.
- Owner boundaries should be explicit and parallel, not hidden behind facades.
- catalog owns static truth; runtime owns dynamic assembly/route/instantiation; runtime_registry owns machine-readable registry snapshot and semantic lookup; providers own upstream protocol adapters.
- Transport/policy concerns should stay as explicit machine-readable contracts, not leak back into provider-specific special cases.
- Avoid compatibility noise and avoid turning runtime into a god-assembler.
- Prefer owner-local tactical fixes that do not leak instability to higher callers.
