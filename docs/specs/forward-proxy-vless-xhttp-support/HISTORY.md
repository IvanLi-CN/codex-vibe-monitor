# VLESS XHTTP Forward Proxy Support 主题历史

> 这里记录主题局部生命周期、替换、兼容性与必要背景；完整 ADR 取舍保留在 `docs/adr/`。单次任务流水账不放这里，规范正文仍以 `./SPEC.md` 为准。

## Lifecycle / Compatibility

- New active topic for additive VLESS XHTTP support.
- Existing forward-proxy protocols and their identities remain compatibility boundaries.

## Replacements / Background

- The deployed forward-proxy validator accepts the VLESS share-link shape but the Xray builder only emits transport-specific settings for TCP, WS, gRPC, and HTTPUpgrade, leaving XHTTP-specific settings absent.
- The topic was created to make XHTTP configuration generation, validation, runtime routing, identity, and diagnostics one durable contract instead of treating a failed XHTTP node as a generic unavailable candidate.

## Implemented Compatibility Boundary

- VLESS XHTTP now builds Xray `xhttpSettings` from `type/net`, top-level transport fields, and a validated JSON object nested under `xhttpSettings.extra`; top-level `host/path/mode` override same-name extra fields, and VMess/Trojan do not inherit the capability implicitly.
- Manual imports, subscriptions, saved nodes, single validation, batch validation, and latency probes continue through the shared outbound builder and preserve the existing success-status and batch-round contracts.
- Canonical identity now consumes the effective XHTTP configuration with recursive object-key ordering, while existing non-XHTTP transport and binding aliases remain unchanged.
- Validation responses retain their existing fields and add stable failure codes with bounded, redacted messages; Settings preserves those codes, and binding-node aliases expose only opaque keys while internal URL aliases remain available for compatibility; test fixtures contain no user-provided node credentials.
- Xray now preflights generated configuration before starting a route, uses exclusive `0600` config creation, and the owner-facing Settings success state keeps normalized links internal for persistence without rendering credentials; runtime/statistics endpoint fields are reduced to protocol/host/port identifiers, XHTTP latency status handling is strict while legacy protocol latency behavior remains compatible.
- Local Rust and web verification passed; an earlier controlled Xray smoke accepted the redacted XHTTP/Reality route and returned an allowed upstream 404. A final-submission retry passed config preflight and reached XHTTP dialing but the remote endpoint closed with EOF/000, so that retry is recorded as unavailable rather than success; release delivery still requires the final review and PR gates.

## Related Changes

- `./IMPLEMENTATION.md`

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
