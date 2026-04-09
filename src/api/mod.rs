// Thin API entry for reviewability; behavior is preserved via include! slices.

// Invocations + summary/stat aggregation + list/detail handlers.
include!("slices/invocations_and_summary.rs");
// Prompt-cache conversation queries + timeseries/public stats windows.
include!("slices/prompt_cache_and_timeseries.rs");
// Error/failure distribution handlers + SSE broadcast stream.
include!("slices/error_distribution_and_sse.rs");
// Settings/model/pricing endpoints + shared API shapes/cache structs.
include!("slices/settings_models_and_cache.rs");
