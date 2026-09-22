# ADR 0016: Autonomous raw capture circuit breaker

## Status

Accepted

## Decision

Raw request and response bodies are diagnostics, not a prerequisite for proxy delivery or structured invocation accounting. When the physical raw store reaches its configured high watermark, or the filesystem reaches its configured low-free-space watermark, the service will stop creating new raw payload files while continuing the proxy and durable structured records.

The circuit breaker is hysteretic: it reopens only after the raw store and filesystem both cross their recovery watermarks. Its decision uses a durable physical-byte inventory plus bounded in-flight reservations, never a request-path directory walk or logical payload-byte estimate.

The accepted high/low watermarks are 16 GiB / 12 GiB for the physical raw store and 20 GiB / 30 GiB for filesystem available space. Capture remains suppressed until the expired retention backlog is known not to be growing. The additive System Status contract reports `unknown`, `capturing`, or `storage_suppressed` with a low-cardinality reason and sanitized measurements; missing fields remain unknown for older consumers.

Retention recovery remains maintenance-class work behind P1 and interactive proxy writes. It may automatically archive, verify, reuse, quarantine, or remove only artifacts whose safety conditions are proven; it must not restart the service, invoke an operator command, run `VACUUM`, or remove source/raw data merely to make space.
