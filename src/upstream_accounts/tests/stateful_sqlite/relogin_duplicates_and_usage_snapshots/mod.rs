use super::*;
use serde_json::json;

macro_rules! insert_window_actual_usage_invocation {
    (
        $pool:expr,
        $account_id:expr,
        $occurred_at:expr,
        $input_tokens:expr,
        $output_tokens:expr,
        $cache_input_tokens:expr,
        $total_tokens:expr,
        $cost:expr $(,)?
    ) => {{
        super::insert_window_actual_usage_invocation(
            $pool,
            WindowActualUsageInvocation {
                account_id: $account_id,
                occurred_at: $occurred_at,
                input_tokens: $input_tokens,
                output_tokens: $output_tokens,
                cache_input_tokens: $cache_input_tokens,
                total_tokens: $total_tokens,
                cost: $cost,
            },
        )
    }};
}

mod part_01;
mod part_02;
mod part_03;
mod part_04;
mod part_05;

pub(crate) use part_01::*;
pub(crate) use part_02::*;
pub(crate) use part_03::*;
pub(crate) use part_04::*;
pub(crate) use part_05::*;
