# HTTP API Contract

## Account detail

`GET /api/pool/upstream-accounts/:id` continues to return the account detail and adds `modelCatalog`:

```json
{
  "status": "never|refreshing|ready|stale|failed",
  "models": ["gpt-5.5"],
  "lastAttemptedAt": "2026-09-06T00:00:00Z",
  "lastSuccessfulAt": "2026-09-06T00:00:00Z",
  "error": { "code": "upstream_http", "message": "Upstream returned HTTP 502." }
}
```

`models`, timestamps, and `error` are nullable/empty according to the state. `stale` is derived from a successful snapshot older than 24 hours; it does not remove models or trigger a request. Error messages are safe summaries.

## Explicit refresh

`POST /api/pool/upstream-accounts/:id/models/refresh`

- Requires the same-origin settings-write guard as other account mutations.
- Takes no request body.
- Returns `200` with the refreshed account detail on success.
- Returns `502` for a provider/upstream discovery failure, `400` for an unsupported account kind, `404` when the account does not exist, and `500` for an internal persistence failure.
- A failed request still returns the retained catalog state in the error response when available; it never clears a previous successful snapshot.
- Refreshes for the same account are serialized. A refresh for another account is not blocked by that account's lock.

## Provider boundary

The refresh handler selects an adapter from the account kind. The adapter receives only the account row, decrypted credential material in memory, account upstream URL, and resolved forward-proxy scope. It returns normalized model IDs or a classified safe error. No downstream request headers or raw upstream payload are part of this contract.
