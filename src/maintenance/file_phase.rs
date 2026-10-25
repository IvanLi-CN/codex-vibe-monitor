use anyhow::{Context, Result};
use std::{future::Future, sync::Arc};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

#[derive(Clone)]
pub(crate) struct FilePhasePermit(Arc<OwnedSemaphorePermit>);

#[derive(Clone)]
pub(crate) struct FilePhaseGate {
    semaphore: Arc<Semaphore>,
}

impl FilePhaseGate {
    pub(crate) fn new() -> Self {
        Self {
            semaphore: Arc::new(Semaphore::new(1)),
        }
    }

    async fn acquire(&self) -> FilePhasePermit {
        FilePhasePermit(Arc::new(
            Arc::clone(&self.semaphore)
                .acquire_owned()
                .await
                .expect("file phase gate is never closed"),
        ))
    }

    fn try_acquire(&self) -> Option<FilePhasePermit> {
        Arc::clone(&self.semaphore)
            .try_acquire_owned()
            .ok()
            .map(|permit| FilePhasePermit(Arc::new(permit)))
    }
}

static GLOBAL_FILE_PHASE_GATE: std::sync::OnceLock<FilePhaseGate> = std::sync::OnceLock::new();

fn global_file_phase_gate() -> &'static FilePhaseGate {
    GLOBAL_FILE_PHASE_GATE.get_or_init(FilePhaseGate::new)
}

tokio::task_local! {
    static ACTIVE_FILE_PHASE_PERMIT: FilePhasePermit;
}

pub(crate) async fn with_file_phase_permit<F: Future>(
    permit: FilePhasePermit,
    future: F,
) -> F::Output {
    ACTIVE_FILE_PHASE_PERMIT.scope(permit, future).await
}

pub(crate) async fn spawn_file_phase_task<F, T>(permit: FilePhasePermit, future: F) -> Result<T>
where
    F: Future<Output = Result<T>> + Send + 'static,
    T: Send + 'static,
{
    tokio::spawn(with_file_phase_permit(permit, future))
        .await
        .context("file phase worker task failed")?
}

pub(crate) fn try_current_file_phase_permit() -> Option<FilePhasePermit> {
    ACTIVE_FILE_PHASE_PERMIT.try_with(Clone::clone).ok()
}

pub(crate) fn try_acquire_file_phase_permit() -> Option<FilePhasePermit> {
    try_current_file_phase_permit().or_else(|| global_file_phase_gate().try_acquire())
}

pub(crate) async fn acquire_file_phase_permit(operation: &'static str) -> Result<FilePhasePermit> {
    if let Some(permit) = try_current_file_phase_permit() {
        return Ok(permit);
    }

    let acquire = global_file_phase_gate().acquire();
    if let Some(shutdown) = super::retention::retention_run_shutdown_token() {
        tokio::select! {
            biased;
            _ = shutdown.cancelled() => {
                super::retention::retention_record_defer(operation, "shutdown");
                Err(super::retention::retention_write_deferred(operation))
            }
            permit = acquire => Ok(permit),
        }
    } else {
        Ok(acquire.await)
    }
    .context("acquire shared maintenance file phase permit")
}

pub(crate) async fn acquire_file_phase_permit_for_cleanup() -> FilePhasePermit {
    if let Some(permit) = try_current_file_phase_permit() {
        return permit;
    }
    global_file_phase_gate().acquire().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::sync::Notify;

    #[tokio::test]
    async fn file_phase_gate_is_fifo_and_nested_scopes_reuse_the_owned_permit() {
        let gate = FilePhaseGate::new();
        let holder = gate.acquire().await;
        let order = Arc::new(Mutex::new(Vec::new()));
        let first_ready = Arc::new(Notify::new());
        let second_ready = Arc::new(Notify::new());
        let first_queued = Arc::new(Notify::new());

        let first = tokio::spawn({
            let gate = gate.clone();
            let order = Arc::clone(&order);
            let ready = Arc::clone(&first_ready);
            let queued = Arc::clone(&first_queued);
            async move {
                queued.notify_one();
                let permit = gate.acquire().await;
                order.lock().unwrap().push(1);
                ready.notify_one();
                permit
            }
        });
        first_queued.notified().await;
        let second = tokio::spawn({
            let gate = gate.clone();
            let order = Arc::clone(&order);
            let ready = Arc::clone(&second_ready);
            async move {
                let permit = gate.acquire().await;
                order.lock().unwrap().push(2);
                ready.notify_one();
                permit
            }
        });

        drop(holder);
        first_ready.notified().await;
        drop(first.await.unwrap());
        second_ready.notified().await;
        drop(second.await.unwrap());

        assert_eq!(*order.lock().unwrap(), vec![1, 2]);

        let permit = gate.acquire().await;
        with_file_phase_permit(permit.clone(), async {
            assert!(try_current_file_phase_permit().is_some());
            assert!(gate.try_acquire().is_none());
        })
        .await;
        drop(permit);
        assert!(gate.try_acquire().is_some());
    }

    #[tokio::test]
    async fn cancelled_file_phase_waiter_does_not_consume_the_next_turn() {
        let gate = FilePhaseGate::new();
        let holder = gate.acquire().await;
        let waiter = tokio::spawn({
            let gate = gate.clone();
            async move { gate.acquire().await }
        });
        tokio::task::yield_now().await;
        waiter.abort();
        let _ = waiter.await;
        drop(holder);

        let permit = tokio::time::timeout(std::time::Duration::from_secs(1), gate.acquire())
            .await
            .expect("cancelled waiter must not strand the gate");
        drop(permit);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn native_worker_keeps_file_phase_until_it_exits() {
        struct ReleaseOnDrop(Arc<std::sync::atomic::AtomicBool>);

        impl Drop for ReleaseOnDrop {
            fn drop(&mut self) {
                self.0.store(true, std::sync::atomic::Ordering::Release);
            }
        }

        let gate = FilePhaseGate::new();
        let permit = gate.acquire().await;
        let worker_permit = permit.clone();
        let started = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let release = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _release_on_drop = ReleaseOnDrop(Arc::clone(&release));
        let worker_started = Arc::clone(&started);
        let worker_release = Arc::clone(&release);
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = worker_permit;
            worker_started.store(true, std::sync::atomic::Ordering::Release);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !worker_release.load(std::sync::atomic::Ordering::Acquire)
                && std::time::Instant::now() < deadline
            {
                std::thread::yield_now();
            }
        });

        tokio::time::timeout(std::time::Duration::from_secs(1), async {
            while !started.load(std::sync::atomic::Ordering::Acquire) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("native worker must start");
        drop(permit);
        assert!(gate.try_acquire().is_none());

        release.store(true, std::sync::atomic::Ordering::Release);
        worker.await.unwrap();
        assert!(gate.try_acquire().is_some());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_waiter_detaches_file_phase_drain_task() {
        let gate = FilePhaseGate::new();
        let permit = gate.acquire().await;
        let started = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let worker_started = Arc::clone(&started);
        let worker_release = Arc::clone(&release);
        let waiter = tokio::spawn(spawn_file_phase_task(permit, async move {
            worker_started.notify_one();
            worker_release.notified().await;
            Ok::<(), anyhow::Error>(())
        }));

        started.notified().await;
        waiter.abort();
        let _ = waiter.await;
        assert!(gate.try_acquire().is_none());

        release.notify_one();
        let permit = tokio::time::timeout(std::time::Duration::from_secs(1), gate.acquire())
            .await
            .expect("detached file phase worker must release after draining");
        drop(permit);
    }
}
