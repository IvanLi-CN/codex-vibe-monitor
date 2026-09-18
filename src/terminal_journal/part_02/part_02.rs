impl TerminalJournal {
    fn take_replay_chunk(
        &mut self,
        max_writes: usize,
        max_bytes: usize,
    ) -> Vec<BatchedTerminalInvocationWrite> {
        if self.replay_blocked {
            return Vec::new();
        }
        let mut writes = Vec::new();
        let mut estimated_bytes = 0_usize;
        let mut scanned_bytes = 0_usize;
        let mut scanned_lines = 0_usize;
        while can_scan_replay(writes.len(), max_writes, scanned_bytes, scanned_lines) {
            let Some(first_sequence) = self
                .replay_segments
                .front()
                .map(|cursor| cursor.first_sequence)
            else {
                break;
            };
            let Some(segment) = self.segments.get(&first_sequence) else {
                self.replay_segments.pop_front();
                continue;
            };
            let result = read_replay_segment(
                self.replay_segments
                    .front_mut()
                    .expect("replay cursor exists"),
                segment,
                !writes.is_empty(),
                max_bytes,
                &mut estimated_bytes,
                &mut scanned_bytes,
                &mut scanned_lines,
            );
            match result {
                ReplaySegmentResult::Write(write) => writes.push(write),
                ReplaySegmentResult::ReachedEnd => {
                    self.replay_segments.pop_front();
                }
                ReplaySegmentResult::Blocked => {
                    self.replay_blocked = true;
                    break;
                }
                ReplaySegmentResult::Stopped => break,
            }
        }
        writes
    }
}

enum ReplaySegmentResult {
    Write(BatchedTerminalInvocationWrite),
    ReachedEnd,
    Blocked,
    Stopped,
}

fn can_scan_replay(
    write_count: usize,
    max_writes: usize,
    scanned_bytes: usize,
    scanned_lines: usize,
) -> bool {
    write_count < max_writes
        && scanned_bytes < TERMINAL_JOURNAL_REPLAY_SCAN_MAX_BYTES
        && scanned_lines < TERMINAL_JOURNAL_REPLAY_SCAN_MAX_LINES
}

fn read_replay_segment(
    cursor: &mut JournalReplayCursor,
    segment: &JournalSegment,
    has_writes: bool,
    max_bytes: usize,
    estimated_bytes: &mut usize,
    scanned_bytes: &mut usize,
    scanned_lines: &mut usize,
) -> ReplaySegmentResult {
    let Ok(file) = File::open(&segment.path) else {
        return ReplaySegmentResult::Stopped;
    };
    let mut reader = BufReader::new(file);
    if reader.seek(SeekFrom::Start(cursor.byte_offset)).is_err() {
        return ReplaySegmentResult::Stopped;
    }
    loop {
        let stream_start = cursor.byte_offset;
        let parsed = {
            let mut budget_reader = ReplayBudgetReader {
                reader: &mut reader,
                remaining: TERMINAL_JOURNAL_REPLAY_MAX_BYTES,
            };
            serde_json::Deserializer::from_reader(&mut budget_reader)
                .into_iter::<JournalLine>()
                .next()
        };
        let Ok(stream_end) = reader.stream_position() else {
            return ReplaySegmentResult::Stopped;
        };
        if stream_end == stream_start {
            return ReplaySegmentResult::ReachedEnd;
        }
        let Some(parsed) = parsed else {
            cursor.byte_offset = stream_end;
            return ReplaySegmentResult::ReachedEnd;
        };
        *scanned_bytes = scanned_bytes.saturating_add(
            stream_end
                .saturating_sub(stream_start)
                .min(usize::MAX as u64) as usize,
        );
        *scanned_lines = scanned_lines.saturating_add(1);
        let Ok(parsed) = parsed else {
            cursor.byte_offset = stream_start;
            warn!(
                replay_offset = stream_start,
                "terminal journal replay stopped at an unparseable entry; preserving the cursor for recovery"
            );
            return ReplaySegmentResult::Blocked;
        };
        cursor.byte_offset = stream_end;
        let Some(entry) = parsed.entry else {
            if can_scan_replay(0, 1, *scanned_bytes, *scanned_lines) {
                continue;
            }
            return ReplaySegmentResult::Stopped;
        };
        if segment.acknowledged.contains(&entry.sequence) {
            if can_scan_replay(0, 1, *scanned_bytes, *scanned_lines) {
                continue;
            }
            return ReplaySegmentResult::Stopped;
        }
        let record = entry.record;
        let write = BatchedTerminalInvocationWrite {
            capture_started: entry.capture_elapsed_ms.and_then(|elapsed_ms| {
                Instant::now().checked_sub(Duration::from_millis(elapsed_ms))
            }),
            raw_capture: entry.raw_capture,
            dashboard_terminal_sequence: Some(0),
            terminal_projection_event_ids: Vec::new(),
            startup_backfill_tasks: startup_backfill_tasks_for_terminal(
                &api_invocation_from_runtime_record(&record),
            ),
            record,
        };
        let write_bytes = write.estimated_memory_bytes();
        if has_writes && estimated_bytes.saturating_add(write_bytes) > max_bytes {
            cursor.byte_offset = stream_start;
            return ReplaySegmentResult::Stopped;
        }
        *estimated_bytes = estimated_bytes.saturating_add(write_bytes);
        return ReplaySegmentResult::Write(write);
    }
}
