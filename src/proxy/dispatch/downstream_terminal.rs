use super::*;
pub(crate) const PROXY_DOWNSTREAM_WRITE_ERROR_GRACE_PERIOD: Duration = Duration::from_secs(2);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DownstreamBodyTerminalState {
    Open,
    SuccessfulTerminalForwarded,
    SuccessfulTerminalCompleted,
    Completed,
    Dropped,
}

pub(crate) struct DownstreamResponseChunk {
    pub(crate) bytes: Bytes,
    pub(crate) successful_terminal: bool,
}

pub(crate) struct TrackedDownstreamReceiverStream {
    inner: ReceiverStream<Result<DownstreamResponseChunk, io::Error>>,
    terminal_tx: watch::Sender<DownstreamBodyTerminalState>,
    terminal_state: DownstreamBodyTerminalState,
}

impl TrackedDownstreamReceiverStream {
    pub(crate) fn new(
        inner: ReceiverStream<Result<DownstreamResponseChunk, io::Error>>,
        terminal_tx: watch::Sender<DownstreamBodyTerminalState>,
    ) -> Self {
        Self {
            inner,
            terminal_tx,
            terminal_state: DownstreamBodyTerminalState::Open,
        }
    }

    fn mark_terminal(&mut self, state: DownstreamBodyTerminalState) {
        let state = if self.terminal_state
            == DownstreamBodyTerminalState::SuccessfulTerminalForwarded
            && state == DownstreamBodyTerminalState::Completed
        {
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        } else {
            state
        };
        let should_update = matches!(
            (self.terminal_state, state),
            (
                DownstreamBodyTerminalState::Open,
                DownstreamBodyTerminalState::SuccessfulTerminalForwarded
                    | DownstreamBodyTerminalState::Completed
                    | DownstreamBodyTerminalState::Dropped
            ) | (
                DownstreamBodyTerminalState::SuccessfulTerminalForwarded,
                DownstreamBodyTerminalState::SuccessfulTerminalCompleted
                    | DownstreamBodyTerminalState::Dropped
            )
        );
        if should_update {
            self.terminal_state = state;
            let _ = self.terminal_tx.send(state);
        }
    }
}

impl futures_util::Stream for TrackedDownstreamReceiverStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner).poll_next(cx) {
            std::task::Poll::Ready(Some(Ok(chunk))) => {
                if chunk.successful_terminal {
                    self.mark_terminal(DownstreamBodyTerminalState::SuccessfulTerminalForwarded);
                }
                std::task::Poll::Ready(Some(Ok(chunk.bytes)))
            }
            std::task::Poll::Ready(Some(Err(err))) => std::task::Poll::Ready(Some(Err(err))),
            std::task::Poll::Ready(None) => {
                self.mark_terminal(DownstreamBodyTerminalState::Completed);
                std::task::Poll::Ready(None)
            }
            std::task::Poll::Pending => std::task::Poll::Pending,
        }
    }
}

impl Drop for TrackedDownstreamReceiverStream {
    fn drop(&mut self) {
        if self.terminal_state == DownstreamBodyTerminalState::SuccessfulTerminalForwarded {
            self.mark_terminal(DownstreamBodyTerminalState::Completed);
        } else {
            self.mark_terminal(DownstreamBodyTerminalState::Dropped);
        }
    }
}

pub(crate) fn proxy_stream_observe_downstream_body_terminal(
    state: DownstreamBodyTerminalState,
    successful_terminal_forwarded: &mut bool,
    downstream_body_available: &mut bool,
    downstream_closed: &mut bool,
    downstream_write_error_kind: &mut Option<&'static str>,
    last_upstream_chunk_received_at: Option<Instant>,
    last_upstream_chunk_gap_ms: &mut Option<u64>,
) {
    match state {
        DownstreamBodyTerminalState::Open
        | DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        | DownstreamBodyTerminalState::Completed => {}
        DownstreamBodyTerminalState::SuccessfulTerminalForwarded => {
            *successful_terminal_forwarded = true;
        }
        DownstreamBodyTerminalState::Dropped => {
            *downstream_body_available = false;
            *downstream_closed = true;
            downstream_write_error_kind.get_or_insert("body_dropped");
            if last_upstream_chunk_gap_ms.is_none() {
                *last_upstream_chunk_gap_ms = last_upstream_chunk_received_at
                    .map(|instant| instant.elapsed().as_millis() as u64);
            }
        }
    }
}

pub(crate) fn proxy_stream_write_error_is_post_terminal(
    successful_terminal_forwarded: bool,
) -> bool {
    successful_terminal_forwarded
}

pub(crate) async fn wait_for_downstream_body_terminal_until(
    downstream_body_terminal_rx: &mut watch::Receiver<DownstreamBodyTerminalState>,
    deadline: Instant,
    successful_terminal_forwarded: &mut bool,
    downstream_body_available: &mut bool,
    downstream_closed: &mut bool,
    downstream_write_error_kind: &mut Option<&'static str>,
    last_upstream_chunk_received_at: Option<Instant>,
    last_upstream_chunk_gap_ms: &mut Option<u64>,
) {
    loop {
        let state = *downstream_body_terminal_rx.borrow_and_update();
        match state {
            DownstreamBodyTerminalState::Open => {
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if tokio::time::timeout(remaining, downstream_body_terminal_rx.changed())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            DownstreamBodyTerminalState::SuccessfulTerminalForwarded => {
                *successful_terminal_forwarded = true;
                let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                    break;
                };
                if tokio::time::timeout(remaining, downstream_body_terminal_rx.changed())
                    .await
                    .is_err()
                {
                    break;
                }
            }
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted => {
                *successful_terminal_forwarded = true;
                break;
            }
            DownstreamBodyTerminalState::Completed => break,
            DownstreamBodyTerminalState::Dropped => {
                proxy_stream_observe_downstream_body_terminal(
                    state,
                    successful_terminal_forwarded,
                    downstream_body_available,
                    downstream_closed,
                    downstream_write_error_kind,
                    last_upstream_chunk_received_at,
                    last_upstream_chunk_gap_ms,
                );
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_until_times_out_when_body_stays_open() {
        let (_tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        tokio::time::timeout(
            Duration::from_millis(200),
            wait_for_downstream_body_terminal_until(
                &mut rx,
                Instant::now() + Duration::from_millis(25),
                &mut false,
                &mut downstream_body_available,
                &mut downstream_closed,
                &mut downstream_write_error_kind,
                None,
                &mut last_upstream_chunk_gap_ms,
            ),
        )
        .await
        .expect("body-terminal wait should not hang forever");
        assert!(!downstream_closed);
        assert!(downstream_body_available);
        assert!(downstream_write_error_kind.is_none());
        assert!(last_upstream_chunk_gap_ms.is_none());
    }

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_until_marks_dropped_before_deadline() {
        let (tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;
        let send_drop = crate::observability::diagnostics::spawn(async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            let _ = tx.send(DownstreamBodyTerminalState::Dropped);
        });

        wait_for_downstream_body_terminal_until(
            &mut rx,
            Instant::now() + Duration::from_millis(200),
            &mut false,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;
        send_drop.await.expect("join drop sender");
        assert!(!downstream_body_available);
        assert!(downstream_closed);
        assert_eq!(downstream_write_error_kind, Some("body_dropped"));
    }

    #[tokio::test]
    async fn downstream_terminal_chunk_preserves_success_latch_when_body_is_released() {
        let (terminal_tx, mut terminal_rx) = watch::channel(DownstreamBodyTerminalState::Open);
        let (response_tx, response_rx) = mpsc::channel(1);
        let mut stream =
            TrackedDownstreamReceiverStream::new(ReceiverStream::new(response_rx), terminal_tx);
        let mut successful_terminal_forwarded = false;
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        response_tx
            .send(Ok(DownstreamResponseChunk {
                bytes: Bytes::from_static(b"event: response.completed\n\n"),
                successful_terminal: true,
            }))
            .await
            .expect("queue successful terminal chunk");
        let chunk = stream
            .next()
            .await
            .expect("terminal chunk should reach downstream body")
            .expect("terminal chunk should not be an error");
        assert!(chunk.starts_with(b"event: response.completed"));
        drop(stream);

        assert_eq!(
            *terminal_rx.borrow_and_update(),
            DownstreamBodyTerminalState::SuccessfulTerminalCompleted
        );
        wait_for_downstream_body_terminal_until(
            &mut terminal_rx,
            Instant::now() + Duration::from_millis(25),
            &mut successful_terminal_forwarded,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;

        assert!(successful_terminal_forwarded);
        assert!(downstream_body_available);
        assert!(!downstream_closed);
        assert!(downstream_write_error_kind.is_none());
    }

    #[tokio::test]
    async fn wait_for_downstream_body_terminal_marks_drop_before_successful_terminal() {
        let (tx, mut rx) = watch::channel(DownstreamBodyTerminalState::Dropped);
        let mut downstream_body_available = true;
        let mut downstream_closed = false;
        let mut downstream_write_error_kind = None;
        let mut last_upstream_chunk_gap_ms = None;

        wait_for_downstream_body_terminal_until(
            &mut rx,
            Instant::now() + Duration::from_millis(25),
            &mut true,
            &mut downstream_body_available,
            &mut downstream_closed,
            &mut downstream_write_error_kind,
            Some(Instant::now()),
            &mut last_upstream_chunk_gap_ms,
        )
        .await;

        drop(tx);
        assert!(!downstream_body_available);
        assert!(downstream_closed);
        assert_eq!(downstream_write_error_kind, Some("body_dropped"));
        assert!(last_upstream_chunk_gap_ms.is_some());
    }

    #[test]
    fn downstream_write_error_is_post_terminal_only_after_successful_terminal_forwarding() {
        assert!(!proxy_stream_write_error_is_post_terminal(false));
        assert!(proxy_stream_write_error_is_post_terminal(true));
    }
}
