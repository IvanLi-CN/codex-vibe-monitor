// Thin proxy entry for reviewability; behavior is preserved via include! slices.
// NOTE: proxy sections currently live under src/proxy/ to avoid touching non-owned call-sites.

// Request entry/admission, replay snapshot preparation, and common pool helpers.
include!("proxy/section_01.rs");
// Pool failover loop, sticky-owner preservation, and upstream attempt bookkeeping.
include!("proxy/section_02.rs");
// Pool routing resolution, live-forward first-attempt path, and inner dispatch glue.
include!("proxy/section_03.rs");
// Capture-target dispatch + request-body limit/read pipeline.
include!("proxy/section_04.rs");
// Timeout budgets, stream gate heuristics, and early payload classifiers.
include!("proxy/section_05.rs");
// Stream rebuild, usage/service-tier extraction, and rollup helpers.
include!("proxy/section_06.rs");
// Capture persistence, raw payload writer paths, and backfill orchestration.
include!("proxy/section_07.rs");
// URL/CORS/request-identity utilities and model payload merge helpers.
include!("proxy/section_08.rs");
