#[cfg(test)]
mod request_compression_query_tests {
    use super::*;
    use crate::tests::{
        SeedInvocationArchiveBatchRow, assert_f64_close, seed_invocation_archive_batch,
        seed_invocation_archive_batch_with_details,
    };
    use sqlx::sqlite::SqlitePoolOptions;

    include!("request_compression_query_part_1.rs");
    include!("request_compression_query_part_2.rs");
    include!("request_compression_query_part_3.rs");
    include!("request_compression_query_part_4.rs");
    include!("request_compression_query_part_5.rs");
    include!("request_compression_query_part_6.rs");
    include!("request_compression_query_part_7.rs");
}
