use std::{
    collections::HashMap,
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
};

use tokio::sync::broadcast;

use crate::{
    ApiInvocation, PromptCacheConversationInvocationPreviewResponse,
    prompt_cache_invocation_preview_from_runtime_record,
};

pub(crate) const RUNTIME_MUTATION_BUS_CAPACITY: usize = 4_096;
pub(crate) const RUNTIME_MUTATION_ROUTER_MAX_BATCH: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RuntimeMutationKind {
    RuntimeUpsert,
    RuntimeRemoved,
    LifecyclePhase,
    TerminalCommitted,
    TerminalPersistedAck,
    Recovery,
}

impl RuntimeMutationKind {
    fn precedence(self) -> u8 {
        match self {
            Self::RuntimeUpsert => 0,
            Self::LifecyclePhase => 1,
            Self::RuntimeRemoved => 2,
            Self::TerminalCommitted => 3,
            Self::TerminalPersistedAck => 4,
            Self::Recovery => 5,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct RuntimeInvocationIdentity {
    pub(crate) invoke_id: String,
    pub(crate) occurred_at: String,
}

impl RuntimeInvocationIdentity {
    pub(crate) fn new(invoke_id: impl Into<String>, occurred_at: impl Into<String>) -> Self {
        Self {
            invoke_id: invoke_id.into(),
            occurred_at: occurred_at.into(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RuntimeInvocationMutation {
    pub(crate) identity: RuntimeInvocationIdentity,
    pub(crate) kind: RuntimeMutationKind,
    pub(crate) row_id: Option<i64>,
    pub(crate) is_terminal: bool,
    pub(crate) prompt_cache_key: Option<String>,
    pub(crate) sticky_key: Option<String>,
    pub(crate) upstream_account_id: Option<i64>,
    pub(crate) preview: Option<Box<PromptCacheConversationInvocationPreviewResponse>>,
}

impl RuntimeInvocationMutation {
    pub(crate) fn from_record(record: &ApiInvocation, kind: RuntimeMutationKind) -> Self {
        let prompt_cache_key = normalize_key(record.prompt_cache_key.as_deref());
        let sticky_key = normalize_key(record.sticky_key.as_deref());
        let preview = prompt_cache_key
            .clone()
            .or_else(|| sticky_key.clone())
            .map(|key| {
                Box::new(prompt_cache_invocation_preview_from_runtime_record(
                    record, key,
                ))
            });
        Self {
            identity: RuntimeInvocationIdentity::new(
                record.invoke_id.clone(),
                record.occurred_at.clone(),
            ),
            kind,
            row_id: (record.id > 0).then_some(record.id),
            is_terminal: crate::app_state::runtime_store_record_is_terminal(record),
            prompt_cache_key,
            sticky_key,
            upstream_account_id: record.upstream_account_id,
            preview,
        }
    }

    pub(crate) fn persisted_ack(
        invoke_id: impl Into<String>,
        occurred_at: impl Into<String>,
        row_id: i64,
    ) -> Self {
        Self {
            identity: RuntimeInvocationIdentity::new(invoke_id, occurred_at),
            kind: RuntimeMutationKind::TerminalPersistedAck,
            row_id: Some(row_id),
            is_terminal: true,
            prompt_cache_key: None,
            sticky_key: None,
            upstream_account_id: None,
            preview: None,
        }
    }

    fn merge_from(&mut self, next: Self) {
        if next.kind.precedence() >= self.kind.precedence() {
            self.kind = next.kind;
            self.is_terminal = next.is_terminal;
        }
        self.row_id = next.row_id.or(self.row_id);
        self.prompt_cache_key = next.prompt_cache_key.or(self.prompt_cache_key.take());
        self.sticky_key = next.sticky_key.or(self.sticky_key.take());
        self.upstream_account_id = next.upstream_account_id.or(self.upstream_account_id);
        self.preview = next.preview.or(self.preview.take());
    }
}

#[derive(Debug, Clone)]
pub(crate) enum RuntimeMutation {
    Invocation(RuntimeInvocationMutation),
    AttemptChanged {
        invoke_id: String,
    },
    PromptCacheBindingChanged {
        prompt_cache_key: String,
    },
    StickyRouteChanged {
        sticky_key: String,
        previous_upstream_account_id: i64,
        upstream_account_id: i64,
    },
}

impl RuntimeMutation {
    pub(crate) fn invocation(record: &ApiInvocation, kind: RuntimeMutationKind) -> Self {
        Self::Invocation(RuntimeInvocationMutation::from_record(record, kind))
    }

    pub(crate) fn terminal_persisted_ack(
        invoke_id: impl Into<String>,
        occurred_at: impl Into<String>,
        row_id: i64,
    ) -> Self {
        Self::Invocation(RuntimeInvocationMutation::persisted_ack(
            invoke_id,
            occurred_at,
            row_id,
        ))
    }

    pub(crate) fn is_invocation(&self) -> bool {
        matches!(self, Self::Invocation(_))
    }

    pub(crate) fn is_terminal_invocation(&self) -> bool {
        matches!(self, Self::Invocation(mutation) if mutation.is_terminal)
    }

    fn key(&self) -> RuntimeMutationKey {
        match self {
            Self::Invocation(mutation) => RuntimeMutationKey::Invocation(mutation.identity.clone()),
            Self::AttemptChanged { invoke_id } => RuntimeMutationKey::Attempt(invoke_id.clone()),
            Self::PromptCacheBindingChanged { prompt_cache_key } => {
                RuntimeMutationKey::Binding(prompt_cache_key.clone())
            }
            Self::StickyRouteChanged {
                sticky_key,
                previous_upstream_account_id,
                upstream_account_id,
            } => RuntimeMutationKey::StickyRoute(
                sticky_key.clone(),
                *previous_upstream_account_id,
                *upstream_account_id,
            ),
        }
    }

    fn merge_from(&mut self, next: Self) {
        match (self, next) {
            (Self::Invocation(current), Self::Invocation(next)) => current.merge_from(next),
            (current, next) => *current = next,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RuntimeMutationKey {
    Invocation(RuntimeInvocationIdentity),
    Attempt(String),
    Binding(String),
    StickyRoute(String, i64, i64),
}

#[derive(Debug, Clone)]
pub(crate) struct SequencedRuntimeMutation {
    pub(crate) sequence: u64,
    pub(crate) mutation: RuntimeMutation,
}

pub(crate) fn coalesce_runtime_mutations(
    mutations: impl IntoIterator<Item = SequencedRuntimeMutation>,
) -> Vec<SequencedRuntimeMutation> {
    let mut indices: HashMap<RuntimeMutationKey, usize> = HashMap::new();
    let mut coalesced: Vec<SequencedRuntimeMutation> = Vec::new();
    for mutation in mutations {
        let key = mutation.mutation.key();
        if let Some(index) = indices.get(&key).copied() {
            let current = &mut coalesced[index];
            current.sequence = current.sequence.max(mutation.sequence);
            current.mutation.merge_from(mutation.mutation);
        } else {
            indices.insert(key, coalesced.len());
            coalesced.push(mutation);
        }
    }
    coalesced.sort_by_key(|mutation| mutation.sequence);
    coalesced
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct RuntimeMutationBusHealth {
    pub(crate) published_count: u64,
    pub(crate) router_lagged_count: u64,
    pub(crate) router_gap_count: u64,
}

#[derive(Debug)]
pub(crate) struct RuntimeMutationBus {
    sender: broadcast::Sender<SequencedRuntimeMutation>,
    next_sequence: AtomicU64,
    published_count: AtomicU64,
    router_lagged_count: AtomicU64,
    router_gap_count: AtomicU64,
    router_started: AtomicBool,
}

impl RuntimeMutationBus {
    pub(crate) fn new() -> Self {
        let (sender, _receiver) = broadcast::channel(RUNTIME_MUTATION_BUS_CAPACITY);
        Self {
            sender,
            next_sequence: AtomicU64::new(0),
            published_count: AtomicU64::new(0),
            router_lagged_count: AtomicU64::new(0),
            router_gap_count: AtomicU64::new(0),
            router_started: AtomicBool::new(false),
        }
    }

    pub(crate) fn publish(&self, mutation: RuntimeMutation) {
        let sequence = self.next_sequence.fetch_add(1, Ordering::Relaxed) + 1;
        self.published_count.fetch_add(1, Ordering::Relaxed);
        // The runtime router owns the only receiver. Sending is non-blocking; if shutdown has
        // already removed it there is no owner-facing consumer left to recover.
        let _ = self
            .sender
            .send(SequencedRuntimeMutation { sequence, mutation });
    }

    pub(crate) fn subscribe(&self) -> broadcast::Receiver<SequencedRuntimeMutation> {
        self.sender.subscribe()
    }

    pub(crate) fn claim_router(&self) -> bool {
        self.router_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub(crate) fn record_router_lag(&self) {
        self.router_lagged_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_router_gap(&self) {
        self.router_gap_count.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn health(&self) -> RuntimeMutationBusHealth {
        RuntimeMutationBusHealth {
            published_count: self.published_count.load(Ordering::Relaxed),
            router_lagged_count: self.router_lagged_count.load(Ordering::Relaxed),
            router_gap_count: self.router_gap_count.load(Ordering::Relaxed),
        }
    }
}

impl Default for RuntimeMutationBus {
    fn default() -> Self {
        Self::new()
    }
}

fn normalize_key(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coalescing_prefers_persisted_terminal_ack_without_retaining_full_records() {
        let identity = RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00");
        let mutations = coalesce_runtime_mutations([
            SequencedRuntimeMutation {
                sequence: 1,
                mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                    identity: identity.clone(),
                    kind: RuntimeMutationKind::RuntimeUpsert,
                    row_id: None,
                    is_terminal: false,
                    prompt_cache_key: None,
                    sticky_key: None,
                    upstream_account_id: None,
                    preview: None,
                }),
            },
            SequencedRuntimeMutation {
                sequence: 2,
                mutation: RuntimeMutation::terminal_persisted_ack(
                    identity.invoke_id,
                    identity.occurred_at,
                    42,
                ),
            },
        ]);

        assert_eq!(mutations.len(), 1);
        let RuntimeMutation::Invocation(mutation) = &mutations[0].mutation else {
            panic!("expected invocation mutation");
        };
        assert_eq!(mutation.kind, RuntimeMutationKind::TerminalPersistedAck);
        assert_eq!(mutation.row_id, Some(42));
        assert!(mutation.is_terminal);
    }

    #[test]
    fn ten_thousand_runtime_mutations_coalesce_by_identity() {
        let mutations = (0..10_000).map(|sequence| SequencedRuntimeMutation {
            sequence: sequence + 1,
            mutation: RuntimeMutation::Invocation(RuntimeInvocationMutation {
                identity: RuntimeInvocationIdentity::new("invoke-1", "2026-08-09 12:00:00"),
                kind: RuntimeMutationKind::LifecyclePhase,
                row_id: None,
                is_terminal: false,
                prompt_cache_key: None,
                sticky_key: None,
                upstream_account_id: None,
                preview: None,
            }),
        });

        let coalesced = coalesce_runtime_mutations(mutations);
        assert_eq!(coalesced.len(), 1);
        assert_eq!(coalesced[0].sequence, 10_000);
    }

    #[tokio::test]
    async fn capacity_overflow_is_non_blocking_and_observable() {
        let bus = RuntimeMutationBus::new();
        let mut receiver = bus.subscribe();

        for index in 0..=RUNTIME_MUTATION_BUS_CAPACITY {
            bus.publish(RuntimeMutation::AttemptChanged {
                invoke_id: format!("invoke-{index}"),
            });
        }

        assert!(matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Lagged(skipped)) if skipped > 0
        ));
        bus.record_router_lag();
        assert_eq!(bus.health().router_lagged_count, 1);
    }
}
