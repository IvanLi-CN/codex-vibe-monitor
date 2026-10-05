use std::ops::{Deref, DerefMut};
use std::sync::{LockResult, PoisonError};

/// The disabled branch uses the original lock, including its poisoning contract.
pub(crate) enum DiagnosticMutex<T> {
    Plain(std::sync::Mutex<T>),
    Profiled(hotpath::wrap::std::sync::Mutex<T>),
}
impl<T> std::fmt::Debug for DiagnosticMutex<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DiagnosticMutex")
    }
}
impl<T> DiagnosticMutex<T> {
    pub(crate) fn new(value: T) -> Self {
        if super::PROFILER_ENABLED.load(std::sync::atomic::Ordering::Relaxed) {
            Self::Profiled(hotpath::mutex!(
                std::sync::Mutex::new(value),
                label = "sqlite_coordinator_state"
            ))
        } else {
            Self::Plain(std::sync::Mutex::new(value))
        }
    }
    pub(crate) fn lock(&self) -> LockResult<DiagnosticMutexGuard<'_, T>> {
        match self {
            Self::Plain(lock) => lock
                .lock()
                .map(DiagnosticMutexGuard::Plain)
                .map_err(|error| PoisonError::new(DiagnosticMutexGuard::Plain(error.into_inner()))),
            Self::Profiled(lock) => {
                lock.lock()
                    .map(DiagnosticMutexGuard::Profiled)
                    .map_err(|error| {
                        PoisonError::new(DiagnosticMutexGuard::Profiled(error.into_inner()))
                    })
            }
        }
    }
}
pub(crate) enum DiagnosticMutexGuard<'a, T> {
    Plain(std::sync::MutexGuard<'a, T>),
    Profiled(hotpath::wrap::std::sync::MutexGuard<'a, T>),
}
impl<T> Deref for DiagnosticMutexGuard<'_, T> {
    type Target = T;
    fn deref(&self) -> &T {
        match self {
            Self::Plain(guard) => guard,
            Self::Profiled(guard) => guard,
        }
    }
}
impl<T> DerefMut for DiagnosticMutexGuard<'_, T> {
    fn deref_mut(&mut self) -> &mut T {
        match self {
            Self::Plain(guard) => guard,
            Self::Profiled(guard) => guard,
        }
    }
}
