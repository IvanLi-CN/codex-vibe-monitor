# VLESS XHTTP Forward Proxy Support

> This file is the durable topic requirements contract. Current implementation facts belong in `IMPLEMENTATION.md`; lifecycle and change references belong in `HISTORY.md`.

## Context and Scope

- Context: Forward proxy share-link ingestion already accepts VLESS links, but the Xray outbound builder does not emit the XHTTP transport settings required by VLESS XHTTP nodes, so valid XHTTP candidates fail validation and cannot be routed.
- In scope: Add VLESS XHTTP support for manual entries, subscription entries, saved-node runtime routing, validation, latency tests, stable node identity, and safe diagnostics.
- Out of scope: VMess XHTTP, new non-XHTTP transports, direct or protocol downgrade fallback, UI redesign, changing existing non-XHTTP routing semantics, and storing live node credentials in tests or documentation.

## Terms and Interfaces

- `XHTTP transport`: Xray's `network: "xhttp"` transport, also represented by Xray's `SplitHTTPConfig` implementation.
- `XHTTP share-link`: A VLESS share URL whose `type` or `net` parameter resolves to `xhttp`.
- `Effective XHTTP configuration`: The Xray `xhttpSettings` produced after decoding the share-link fields and applying Xray's precedence rules.
- `Available candidate`: A candidate for which at least one validation round reaches the configured upstream through the candidate's actual XHTTP route and returns an accepted reachability status.
- `Node identity`: The stable key derived from the effective share-link transport identity and used for deduplication, binding, history, and runtime state.
- `Validation interface`: `POST /api/settings/forward-proxy/validate`; its existing response fields remain compatible and gain a stable machine-readable failure code.

## Requirements

### REQ-VLESS-XHTTP-001

- The system MUST accept VLESS share links whose `type` or `net` value resolves to `xhttp` in both manual and subscription ingestion paths.
- Inputs MUST support the existing VLESS security modes `none`, `tls`, and `reality`.
- Existing TCP, WS, gRPC, HTTPUpgrade, and other currently supported protocol behavior MUST remain unchanged.
- Outputs MUST retain the parsed node as a VLESS forward proxy candidate instead of dropping it as an unsupported entry.

### REQ-VLESS-XHTTP-002

- The system MUST preserve XHTTP behavior fields during normalization and persistence, including `host`, `path`, `mode`, `extra`, and any encoded values required by the Xray share-link format.
- The `type` and `net` aliases MUST resolve to the same effective transport value.
- Percent-encoded XHTTP values MUST be decoded exactly once for configuration generation and MUST remain representable in the normalized share link.

### REQ-VLESS-XHTTP-003

- The VLESS Xray outbound MUST emit `network: "xhttp"` and an `xhttpSettings` object.
- Top-level share-link values `host`, `path`, and `mode` MUST map to the corresponding XHTTP settings.
- The supported mode values MUST be `auto`, `packet-up`, `stream-up`, and `stream-one`; an unsupported mode MUST fail with an explicit configuration error.
- A missing mode MUST use Xray's default behavior rather than inventing a different application default.

### REQ-VLESS-XHTTP-004

- `extra` MUST be parsed as a JSON object when present and MUST be passed through as XHTTP extension configuration without silently dropping nested Xray fields.
- Top-level `host`, `path`, and `mode` values MUST take precedence over same-named values supplied inside `extra`, matching Xray's native configuration behavior.
- Malformed JSON, a non-object `extra` value, or an Xray-rejected XHTTP extension MUST fail validation with a diagnostic configuration error.

### REQ-VLESS-XHTTP-005

- VLESS `tls` settings MUST continue to map SNI, fingerprint, ALPN, and insecure-mode fields to the existing TLS configuration.
- VLESS `reality` settings MUST continue to map SNI, fingerprint, public key, short ID, and spider path fields to Reality configuration.
- A share-link `alpn` value MUST be retained for normalization and identity, but MUST NOT be invented as a field inside `realitySettings`, because the current Xray Reality configuration has no ALPN field.
- XHTTP plus Reality MUST use the Xray-supported Reality/XHTTP combination; it MUST NOT be rewritten as legacy Vision, TLS, or another transport.

### REQ-VLESS-XHTTP-006

- Effective XHTTP configuration MUST participate in node identity and deduplication.
- Query parameter order, equivalent percent encoding, `type` versus `net`, and JSON object key order inside `extra` MUST NOT create duplicate identities when they represent the same effective configuration.
- Changes to `host`, `path`, `mode`, or the effective contents of `extra` MUST produce distinct identities when they change the route behavior.
- Existing non-XHTTP node identities and binding aliases MUST remain compatible.

### REQ-VLESS-XHTTP-007

- Manual import, subscription refresh, saved-node routing, single-node validation, batch validation, and latency testing MUST use the same XHTTP outbound construction rules.
- A node that was already stored before XHTTP support was added MUST become routable after upgrade without requiring re-import, provided its stored link is valid.
- No consumer MAY reconstruct a second, partial XHTTP configuration independently of the shared VLESS builder.

### REQ-VLESS-XHTTP-008

- Candidate validation MUST create a temporary Xray route and send the probe through that route; a direct request MUST NOT count as XHTTP validation.
- The existing batch validation contract MUST remain: at most 12 rounds are attempted and any successful round makes the candidate available; all rounds failing makes it unavailable.
- Each successful round MUST reach the configured upstream through XHTTP and accept the existing reachability statuses: success, `401`, `403`, or `404`.
- A candidate MUST NOT be marked available because XHTTP configuration failed, Xray failed to start, the proxy connection failed, or the application used a direct fallback.

### REQ-VLESS-XHTTP-009

- Validation responses MUST retain the existing `ok`, `message`, `normalizedValue`, `discoveredNodes`, and `latencyMs` compatibility fields.
- Failed validation MUST additionally expose a stable machine-readable code covering at least unsupported transport, invalid XHTTP extra, unsupported mode, Xray configuration failure, Xray startup failure, proxy connection failure, timeout, and rejected upstream status.
- User-facing details MUST be actionable but MUST NOT include the UUID, private credentials, complete share URL, or raw secrets from `extra`.
- Server-side diagnostics MAY include deeper Xray error detail only after redaction of credentials and sensitive query values.

### REQ-VLESS-XHTTP-010

- The feature MUST use the configured or bundled Xray binary as the protocol implementation source of truth.
- If that binary cannot build or run XHTTP, the system MUST return an explicit Xray capability/configuration error and MUST NOT downgrade the node to another transport or direct access.
- The feature MUST be covered by deterministic configuration and identity tests, integration validation tests, regression tests for existing protocols, and one controlled live smoke verification using a temporary or redacted test node outside CI.
- Real UUIDs, private share links, and live `extra` payloads MUST NOT be committed to the repository, durable specifications, or CI logs.

## Verification

### VER-VLESS-XHTTP-001

- Method: Run a fixture matrix through VLESS parsing and Xray outbound generation for `type=xhttp` and `net=xhttp`, all three security modes, all four supported modes, `host`, `path`, and a nested `extra` object.
- covers: `REQ-VLESS-XHTTP-001`, `REQ-VLESS-XHTTP-002`, `REQ-VLESS-XHTTP-003`, `REQ-VLESS-XHTTP-004`, `REQ-VLESS-XHTTP-005`
- Pass condition: The generated configuration contains the expected XHTTP fields, applies top-level precedence, rejects malformed or unsupported values, and does not alter non-XHTTP fixture output.

### VER-VLESS-XHTTP-002

- Method: Compare normalized identities for equivalent query order, encoding, aliases, and `extra` key order, then compare identities after changing each behavior-affecting XHTTP field.
- covers: `REQ-VLESS-XHTTP-006`
- Pass condition: Equivalent effective configurations deduplicate, behavior-changing configurations remain distinct, and existing non-XHTTP identity fixtures remain unchanged.

### VER-VLESS-XHTTP-003

- Method: Exercise manual, subscription, saved-node, single-validation, batch-validation, and latency-test paths against the same XHTTP fixture and a controlled Xray-backed route.
- covers: `REQ-VLESS-XHTTP-007`, `REQ-VLESS-XHTTP-008`
- Pass condition: Every path uses the shared XHTTP configuration, successful probes traverse the temporary Xray route, the 12-round result contract is preserved, and direct fallback cannot produce success.

### VER-VLESS-XHTTP-004

- Method: Inject malformed JSON, unsupported modes, Xray startup failures, proxy transport failures, timeouts, and rejected upstream statuses into validation tests.
- covers: `REQ-VLESS-XHTTP-009`, `REQ-VLESS-XHTTP-010`
- Pass condition: Each failure returns the expected stable code and safe message, compatibility fields remain present, no downgrade occurs, and sensitive values are absent from responses and captured logs.

### VER-VLESS-XHTTP-005

- Method: Run the existing forward-proxy regression suite and perform one controlled live smoke validation with a temporary XHTTP node using the deployed Xray binary.
- covers: `REQ-VLESS-XHTTP-001`, `REQ-VLESS-XHTTP-007`, `REQ-VLESS-XHTTP-008`, `REQ-VLESS-XHTTP-010`
- Pass condition: Existing protocol behavior remains green, the live XHTTP route reaches the configured upstream, and no live credential is persisted in repository or CI artifacts.

## Related ADRs

None

## Visual Evidence

- None. This is a backend protocol and validation capability; no visual artifact is normative.

## References

- `src/share_links.rs`
- `src/forward_proxy/slices/manager_xray_types_and_tests.rs`
- `src/forward_proxy/slices/storage_and_hourly_stats/probe_and_validation.rs`
- `web/src/pages/Settings.tsx`
- [Xray transport configuration](https://github.com/XTLS/Xray-core/blob/main/infra/conf/transport_method.go)
- [Xray Reality configuration](https://github.com/XTLS/Xray-core/blob/main/transport/internet/reality/config.proto)
- `./IMPLEMENTATION.md`
- `./HISTORY.md`
