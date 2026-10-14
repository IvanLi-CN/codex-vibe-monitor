use super::*;

pub(crate) async fn finish_proxy_raw_captures(
    request: PendingRawPayloadWrite,
    response: AsyncStreamingRawPayloadWriter,
) -> (RawPayloadMeta, RawPayloadMeta, u64) {
    let request = request.finish().await;
    let started = Instant::now();
    let response = response.finish().await;
    (request, response, started.elapsed().as_millis() as u64)
}
