# Public Blog Runtime API Topic History

> This file records lifecycle, compatibility, and related changes. The normative contract remains in `./SPEC.md`.

## Lifecycle / Compatibility

- The topic introduces one additive public read-only API and leaves existing API contracts unchanged.

## Replacements / Background

- The blog runtime panel consumes aggregate values and does not use invocation-level details.

## Related Changes

- The initial implementation is delivered in the current topic PR. Record its live PR reference here after publication.
- A review follow-up exposes `Retry-After` to allowed browser clients and verifies the cross-origin 429 response. The live PR reference will be recorded after publication.

## References

- `./SPEC.md`
- `./IMPLEMENTATION.md`
