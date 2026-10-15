use super::*;

pub(crate) async fn finish_proxy_raw_captures(
    request: PendingRawPayloadWrite,
    response: AsyncStreamingRawPayloadWriter,
) -> (RawPayloadMeta, RawPayloadMeta, u64) {
    let response = async move {
        let started = Instant::now();
        let meta = response.finish().await;
        (meta, started.elapsed().as_millis() as u64)
    };
    // The response may own the writer slot needed by queued request capture.
    // Poll both finish futures so response EOF releases that slot before waiting.
    let (request, (response, response_elapsed)) = tokio::join!(request.finish(), response);
    (request, response, response_elapsed)
}
