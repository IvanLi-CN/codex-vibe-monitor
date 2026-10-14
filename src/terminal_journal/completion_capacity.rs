use super::*;

// JSON may escape each byte as six characters. Keep terminal memory headroom separate
// from the frame limit: small attempts must not each reserve a maximum-size frame.
const COMPLETION_GROWTH_BYTES: u64 = 64 * 1024 * 6;
const ACK_FRAME_MAX_BYTES: u64 = 256;

#[derive(Debug)]
struct AttemptCredit {
    invocation: (String, String),
    bytes: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_growth_backpressures_before_runtime_publication_and_recovers() {
        let mut snapshot = test_attempt_snapshot();
        let store = crate::RuntimeAttemptStore::default();
        assert!(store.upsert(snapshot.clone()));
        let mut capacity = CompletionCapacity::default();
        capacity.insert(&snapshot);
        let initial = capacity.reserved_bytes();
        let pending = TERMINAL_JOURNAL_MAX_BYTES - initial;
        snapshot.response_raw_path = Some("x".repeat(64 * 1024));
        assert!(
            !store.upsert_with_admission(snapshot.clone(), false, |candidate| {
                capacity.reserve_growth(candidate, pending)
            })
        );
        assert!(
            store
                .get(
                    &snapshot.invoke_id,
                    &snapshot.occurred_at,
                    &snapshot.attempt_public_id
                )
                .unwrap()
                .response_raw_path
                .is_none()
        );
        assert_eq!(capacity.reserved_bytes(), initial);
        assert_eq!(store.health_snapshot().rejected_records, 1);
        assert!(
            store.upsert_with_admission(snapshot.clone(), false, |candidate| {
                capacity.reserve_growth(candidate, pending - 1024 * 1024)
            })
        );
        assert_eq!(
            store
                .get(
                    &snapshot.invoke_id,
                    &snapshot.occurred_at,
                    &snapshot.attempt_public_id
                )
                .unwrap()
                .response_raw_path,
            snapshot.response_raw_path
        );
        assert!(capacity.reserved_bytes() > initial);
    }

    #[test]
    fn retries_share_invocation_credit_until_the_last_attempt_releases() {
        let first = test_attempt_snapshot();
        let mut second = first.clone();
        second.attempt_public_id = "retry-2".into();
        let mut capacity = CompletionCapacity::default();
        capacity.insert(&first);
        let first_total = capacity.reserved_bytes();
        let increment = capacity.start_increment(&second);
        capacity.insert(&second);
        assert_eq!(capacity.reserved_bytes(), first_total + increment);
        assert_eq!(
            capacity.released_bytes(&[&first.attempt_public_id, &first.attempt_public_id]),
            first_total - COMPLETION_GROWTH_BYTES
        );
        capacity.release(&first.attempt_public_id);
        assert_eq!(
            capacity.reserved_bytes(),
            increment + COMPLETION_GROWTH_BYTES
        );
        capacity.release(&second.attempt_public_id);
        assert_eq!(capacity.reserved_bytes(), 0);
    }

    #[test]
    fn escaped_metadata_terminal_growth_and_maximum_ack_fit_completion_credit() {
        let mut snapshot = test_attempt_snapshot();
        snapshot.request_model = Some("\u{0001}".repeat(64 * 1024));
        snapshot.upstream_route_key = format!("https://example.invalid/{}", "猫".repeat(1024));
        snapshot.upstream_account_id = i64::MAX;
        let mut capacity = CompletionCapacity::default();
        capacity.insert(&snapshot);
        // Exercise the growth headroom without acquiring additional credit.
        snapshot.finished_at = Some("2026-07-29T00:00:01Z".into());
        snapshot.status = "success".into();
        snapshot.phase = Some("\u{0001}".repeat(32 * 1024));
        snapshot.response_raw_path = Some("\u{0001}".repeat(16 * 1024));
        let mut record =
            crate::tests::test_proxy_capture_record(&snapshot.invoke_id, &snapshot.occurred_at);
        record.model = Some("\u{0001}".repeat(64 * 1024));
        let entry = JournalTerminalRecord {
            sequence: u64::MAX,
            raw_capture: false,
            capture_elapsed_ms: Some(u64::MAX),
            attempts: vec![journal_safe_attempt_snapshot(&snapshot)],
            record: journal_safe_terminal_record(&record),
        };
        let frame = encode_entry_line(&entry).unwrap();
        let ack = encode_acknowledgement_line(u64::MAX).unwrap();
        assert!(frame.len() < TERMINAL_JOURNAL_REPLAY_MAX_BYTES);
        assert!(ack.len() as u64 <= ACK_FRAME_MAX_BYTES);
        assert!((frame.len() + ack.len()) as u64 <= capacity.reserved_bytes());
    }
}

#[derive(Debug, Default)]
pub(super) struct CompletionCapacity {
    attempts: HashMap<String, AttemptCredit>,
    invocation_counts: HashMap<(String, String), usize>,
    reserved_bytes: u64,
}

impl CompletionCapacity {
    fn attempt_credit(snapshot: &RuntimeAttemptSnapshot) -> u64 {
        // Only persisted metadata contributes. Do not clone redacted request summaries
        // or provider diagnostics on every progress update. 4096 covers schema keys,
        // punctuation, nulls and maximum-width numeric fields.
        let required_strings = [
            snapshot.attempt_public_id.as_str(),
            snapshot.invoke_id.as_str(),
            snapshot.occurred_at.as_str(),
            snapshot.endpoint.as_str(),
            snapshot.started_at.as_str(),
            snapshot.status.as_str(),
        ];
        let optional_strings = [
            &snapshot.routing_control_fingerprint,
            &snapshot.routing_source,
            &snapshot.upstream_base_url_host,
            &snapshot.group_name_snapshot,
            &snapshot.request_model,
            &snapshot.upstream_request_model,
            &snapshot.model_mapping_pattern,
            &snapshot.finished_at,
            &snapshot.phase,
            &snapshot.failure_kind,
            &snapshot.compact_support_status,
            &snapshot.upstream_request_compression_algorithm,
            &snapshot.upstream_request_compression_mode,
            &snapshot.response_raw_path,
            &snapshot.response_raw_codec,
            &snapshot.response_raw_truncated_reason,
            &snapshot.response_content_encoding,
        ];
        let string_bytes = required_strings
            .iter()
            .map(|value| value.len() as u64)
            .sum::<u64>()
            .saturating_add(
                optional_strings
                    .iter()
                    .filter_map(|value| value.as_ref())
                    .map(|value| value.len() as u64)
                    .sum::<u64>(),
            )
            // URL canonicalization can percent-encode every original UTF-8 byte.
            .saturating_add((snapshot.upstream_route_key.len() as u64).saturating_mul(3));
        string_bytes
            .saturating_mul(6)
            .saturating_add(4096)
            .saturating_add(COMPLETION_GROWTH_BYTES)
            .min(TERMINAL_JOURNAL_REPLAY_MAX_BYTES as u64)
            .saturating_add(ACK_FRAME_MAX_BYTES)
    }

    pub(super) fn start_increment(&self, snapshot: &RuntimeAttemptSnapshot) -> u64 {
        let invocation = (snapshot.invoke_id.clone(), snapshot.occurred_at.clone());
        Self::attempt_credit(snapshot).saturating_add(
            if self.invocation_counts.contains_key(&invocation) {
                0
            } else {
                COMPLETION_GROWTH_BYTES
            },
        )
    }

    pub(super) fn insert(&mut self, snapshot: &RuntimeAttemptSnapshot) {
        let invocation = (snapshot.invoke_id.clone(), snapshot.occurred_at.clone());
        let bytes = Self::attempt_credit(snapshot);
        let count = self
            .invocation_counts
            .entry(invocation.clone())
            .or_default();
        if *count == 0 {
            self.reserved_bytes = self.reserved_bytes.saturating_add(COMPLETION_GROWTH_BYTES);
        }
        *count += 1;
        self.reserved_bytes = self.reserved_bytes.saturating_add(bytes);
        self.attempts.insert(
            snapshot.attempt_public_id.clone(),
            AttemptCredit { invocation, bytes },
        );
    }

    pub(super) fn reserve_growth(
        &mut self,
        snapshot: &RuntimeAttemptSnapshot,
        pending_bytes: u64,
    ) -> bool {
        let Some(existing) = self.attempts.get(&snapshot.attempt_public_id) else {
            return false;
        };
        if existing.invocation.0 != snapshot.invoke_id
            || existing.invocation.1 != snapshot.occurred_at
        {
            return false;
        }
        let required = Self::attempt_credit(snapshot);
        let required = required.max(existing.bytes);
        let reserved = self
            .reserved_bytes
            .saturating_sub(existing.bytes)
            .saturating_add(required);
        if pending_bytes.saturating_add(reserved) > TERMINAL_JOURNAL_MAX_BYTES {
            return false;
        }
        self.attempts
            .get_mut(&snapshot.attempt_public_id)
            .expect("credit remains present")
            .bytes = required;
        self.reserved_bytes = reserved;
        true
    }

    pub(super) fn reserved_bytes(&self) -> u64 {
        self.reserved_bytes
    }

    pub(super) fn released_bytes(&self, ids: &[&str]) -> u64 {
        let mut released = 0_u64;
        let mut invocation_counts = HashMap::<&(String, String), usize>::new();
        let mut seen = HashSet::new();
        for id in ids {
            if seen.insert(*id)
                && let Some(credit) = self.attempts.get(*id)
            {
                released = released.saturating_add(credit.bytes);
                *invocation_counts.entry(&credit.invocation).or_default() += 1;
            }
        }
        for (invocation, count) in invocation_counts {
            if self.invocation_counts.get(invocation) == Some(&count) {
                released = released.saturating_add(COMPLETION_GROWTH_BYTES);
            }
        }
        released
    }

    pub(super) fn release(&mut self, id: &str) {
        let Some(credit) = self.attempts.remove(id) else {
            return;
        };
        self.reserved_bytes = self.reserved_bytes.saturating_sub(credit.bytes);
        if let Some(count) = self.invocation_counts.get_mut(&credit.invocation) {
            *count -= 1;
            if *count == 0 {
                self.invocation_counts.remove(&credit.invocation);
                self.reserved_bytes = self.reserved_bytes.saturating_sub(COMPLETION_GROWTH_BYTES);
            }
        }
    }
}
