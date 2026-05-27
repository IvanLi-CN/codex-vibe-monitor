# History

## r4p9x

Account-pool routing policy moved from isolated group/tag behavior to a layered effective policy model. The resolver now computes one effective policy per account and downstream routing code reads that policy instead of separate group or tag fragments.

2026-05-27: Clarified and enforced sticky transfer boundaries: `allow_cut_out=false` blocks automatic timeout/failover migration even when the current route key is excluded, while explicit Prompt Cache bindings remain the only manual cut-out override. HTTP 4xx responses no longer count as sticky route successes.
