# hotpath SQL normalization and redaction patch

`hotpath/` is the published MIT-licensed hotpath 0.28.0 crate. The original file
hashes are recorded in `hotpath-upstream.sha256.json`; Cargo still pins macros and
drain through the project lockfile. Only these upstream source files are patched:

- `src/lib_on/sql.rs`: use the SQL worker's normalization cache.
- `src/lib_on/sql/normalize.rs`: redact SQLite single/double-quoted text, comments,
  hexadecimal/exponent/fractional numeric literals and cache repeated statements,
  capped at 256 entries,
  2 MiB of combined statement/normalized text and 64 KiB per entry. Larger inputs
  use the uncached normalizer. Metadata is bounded by the
  entry count. FIFO eviction never removes aggregate observations.

The registry archive SHA-256 is
`d8345e171844f80e5ae4cbed8bc8276e094f6ff2293a6f8e547c200183976674`.
The crate's VCS commit is `d88396629afc791d043f3a2dfe466d01c8f2835d`;
`hotpath/LICENSE.txt` copies the repository's root MIT license from that same
commit, because the published crate omitted it.
The project's tests compile the exact patched normalization module; the root
dev-dependency on regex-lite is used only by that regression seam.

CPU sampling of the application attributed most profiler cost to repeated regex
normalization. The patch keeps every SQL event, count, duration, percentile,
source and route. Exported templates conservatively redact double-quoted
identifiers as well as literals: SQLite can interpret the same quoted text as
either, depending on schema and connection options. Comments and incomplete
quoted text are also redacted. Unquoted UTF-8 identifiers remain intact. This adds
no global recorder, worker, daemon or production service. Cached raw statements
remain private worker memory and are never exported.

Validate with the cache regression tests and the application's full SQL runtime
acceptance. On a dependency upgrade, compare the two patched files against the
upstream hash manifest, port only these bounded-cache/redaction corrections if
still necessary, and rerun the
same default-on/default-off overhead acceptance. Remove the override when an
upstream release provides equivalent bounded caching and export redaction.
