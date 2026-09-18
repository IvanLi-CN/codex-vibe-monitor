impl TerminalJournal {
    pub(crate) fn quarantine(
        &mut self,
        terminal: &BatchedTerminalInvocationWrite,
        error: &str,
    ) -> Result<()> {
        self.quarantine_terminals(std::slice::from_ref(&terminal), error)
    }

    pub(crate) fn quarantine_terminals(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct QuarantineEntry<'a> {
            invoke_id: &'a str,
            occurred_at: &'a str,
            raw_capture: bool,
            error: &'a str,
            record: &'a ProxyCaptureRecord,
        }

        let path = self.directory.join("quarantine.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open terminal quarantine {}", path.display()))?;
        for terminal in terminals {
            let entry = QuarantineEntry {
                invoke_id: &terminal.record.invoke_id,
                occurred_at: &terminal.record.occurred_at,
                raw_capture: terminal.raw_capture,
                error,
                record: &terminal.record,
            };
            serde_json::to_writer(&mut file, &entry)
                .context("failed to encode terminal quarantine")?;
            file.write_all(b"\n")
                .context("failed to append terminal quarantine delimiter")?;
        }
        if !terminals.is_empty() {
            file.sync_data()
                .context("failed to sync terminal quarantine")?;
        }
        Ok(())
    }

    pub(crate) fn quarantine_system_task_finish(
        &mut self,
        finish: &BatchedSystemTaskFinish,
        error: &str,
    ) -> Result<()> {
        self.quarantine_system_task_finishes(std::slice::from_ref(&finish), error)
    }

    pub(crate) fn quarantine_system_task_finishes(
        &mut self,
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct QuarantineEntry<'a> {
            run_id: i64,
            task_kind: &'static str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            error: &'a str,
        }

        let path = self.directory.join("system-task-quarantine.jsonl");
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("failed to open system-task quarantine {}", path.display()))?;
        let mut appended = false;
        let mut newly_indexed_ids = Vec::new();
        let mut newly_deferred_finishes = Vec::new();
        for finish in finishes {
            if !self.system_task_quarantine_ids.insert(finish.run_id) {
                continue;
            }
            newly_indexed_ids.push(finish.run_id);
            self.replayed_system_task_ids.insert(finish.run_id);
            newly_deferred_finishes.push((*finish).clone());
            let entry = QuarantineEntry {
                run_id: finish.run_id,
                task_kind: finish.task_kind.as_str(),
                trigger_kind: &finish.trigger_kind,
                status: finish.status.as_str(),
                summary: &finish.summary,
                detail: &finish.detail,
                finished_at: &finish.finished_at,
                duration_ms: finish.duration_ms,
                error,
            };
            if let Err(err) = serde_json::to_writer(&mut file, &entry) {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                    self.replayed_system_task_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine");
            }
            if let Err(err) = file.write_all(b"\n") {
                for run_id in newly_indexed_ids {
                    self.system_task_quarantine_ids.remove(&run_id);
                    self.replayed_system_task_ids.remove(&run_id);
                }
                return Err(err).context("failed to append system-task quarantine delimiter");
            }
            appended = true;
        }
        if appended && let Err(err) = file.sync_data() {
            for run_id in newly_indexed_ids {
                self.system_task_quarantine_ids.remove(&run_id);
                self.replayed_system_task_ids.remove(&run_id);
            }
            return Err(err).context("failed to sync system-task quarantine");
        }
        for finish in newly_deferred_finishes {
            self.deferred_system_task_finishes.push_back(finish);
        }
        Ok(())
    }

    pub(crate) fn take_system_task_finishes(&mut self, max: usize) -> Vec<BatchedSystemTaskFinish> {
        self.deferred_system_task_finishes
            .drain(..self.deferred_system_task_finishes.len().min(max))
            .collect()
    }

    pub(crate) fn remove_deferred_system_task_finishes(&mut self, run_ids: &[i64]) {
        let run_ids = run_ids.iter().copied().collect::<HashSet<_>>();
        self.deferred_system_task_finishes
            .retain(|finish| !run_ids.contains(&finish.run_id));
    }

    pub(crate) fn acknowledge_system_task_finishes(&mut self, run_ids: &[i64]) -> Result<()> {
        let acknowledged = run_ids
            .iter()
            .copied()
            .filter(|run_id| self.replayed_system_task_ids.contains(run_id))
            .collect::<Vec<_>>();
        if acknowledged.is_empty() {
            return Ok(());
        }
        let path = self.directory.join("system-task-quarantine.jsonl");
        append_system_task_acknowledgements(&path, &acknowledged)?;
        let shutdown_path = self.directory.join("shutdown-system-task-quarantine.jsonl");
        if shutdown_path.exists() {
            // Shutdown recovery is read alongside the regular quarantine file. Keep an ACK in
            // that source too, otherwise regular-file compaction can erase the only ACK evidence
            // and replay the same task finish after the next restart.
            append_system_task_acknowledgements(&shutdown_path, &acknowledged)?;
        }
        let acknowledged_ids = acknowledged.into_iter().collect::<HashSet<_>>();
        self.replayed_system_task_ids
            .retain(|run_id| !acknowledged_ids.contains(run_id));
        self.deferred_system_task_finishes
            .retain(|finish| !acknowledged_ids.contains(&finish.run_id));
        if let Err(err) = self.compact_system_task_quarantine_if_due() {
            warn!(error = %err, "failed to compact system-task quarantine journal");
        }
        Ok(())
    }

    fn compact_system_task_quarantine_if_due(&mut self) -> Result<()> {
        let path = self.directory.join("system-task-quarantine.jsonl");
        let shutdown_path = self.directory.join("shutdown-system-task-quarantine.jsonl");
        let size = fs::metadata(&path)
            .map(|metadata| metadata.len())
            .unwrap_or(0)
            .max(
                fs::metadata(&shutdown_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(0),
            );
        if size < SYSTEM_TASK_QUARANTINE_COMPACTION_BYTES {
            return Ok(());
        }
        // Rebuild from durable recovery files instead of relying on the in-memory deferred queue.
        // Deterministic failures deliberately remove their payload from memory while retaining
        // the journal record for the next restart; compaction must preserve those records too.
        let recovery_paths = [path.clone(), shutdown_path.clone()];
        let recovery_finishes = load_system_task_finishes(&recovery_paths);
        let recovery_ids = recovery_finishes
            .iter()
            .map(|finish| finish.run_id)
            .collect::<HashSet<_>>();

        let temp_path = path.with_extension("jsonl.tmp");
        let mut file = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&temp_path)
            .with_context(|| {
                format!(
                    "failed to open system-task quarantine temp file {}",
                    temp_path.display()
                )
            })?;
        for finish in &recovery_finishes {
            serde_json::to_writer(
                &mut file,
                &SystemTaskRecoveryLine {
                    run_id: finish.run_id,
                    acknowledged: false,
                    task_kind: finish.task_kind.as_str().to_string(),
                    trigger_kind: finish.trigger_kind.clone(),
                    status: finish.status.as_str().to_string(),
                    summary: finish.summary.clone(),
                    detail: finish.detail.clone(),
                    finished_at: finish.finished_at.clone(),
                    duration_ms: finish.duration_ms,
                    error: None,
                },
            )
            .context("failed to encode compacted system-task quarantine entry")?;
            file.write_all(b"\n")
                .context("failed to append compacted system-task quarantine delimiter")?;
        }
        file.sync_data()
            .context("failed to sync compacted system-task quarantine")?;
        fs::rename(&temp_path, &path).with_context(|| {
            format!(
                "failed to replace system-task quarantine with compacted file {}",
                path.display()
            )
        })?;
        if shutdown_path.exists() {
            let shutdown_temp_path = shutdown_path.with_extension("jsonl.tmp");
            let shutdown_file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&shutdown_temp_path)
                .with_context(|| {
                    format!(
                        "failed to open shutdown system-task quarantine temp file {}",
                        shutdown_temp_path.display()
                    )
                })?;
            shutdown_file
                .sync_data()
                .context("failed to sync compacted shutdown system-task quarantine")?;
            fs::rename(&shutdown_temp_path, &shutdown_path).with_context(|| {
                format!(
                    "failed to replace shutdown system-task quarantine with compacted file {}",
                    shutdown_path.display()
                )
            })?;
        }
        self.system_task_quarantine_ids = recovery_ids;
        Ok(())
    }

    pub(crate) fn quarantine_shutdown_batch(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        Self::quarantine_shutdown_batch_at_directory(&self.directory, terminals, finishes, error)
    }

    pub(crate) fn quarantine_shutdown_batch_at_database_path(
        database_path: &Path,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        let directory = terminal_journal_directory(database_path);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create independent terminal recovery directory {}",
                directory.display()
            )
        })?;
        Self::quarantine_shutdown_batch_at_directory(&directory, terminals, finishes, error)
    }

    fn quarantine_shutdown_batch_at_directory(
        directory: &Path,
        terminals: &[&BatchedTerminalInvocationWrite],
        finishes: &[&BatchedSystemTaskFinish],
        error: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct SystemTaskEntry<'a> {
            run_id: i64,
            task_kind: &'static str,
            trigger_kind: &'a str,
            status: &'static str,
            summary: &'a Option<String>,
            detail: &'a Option<String>,
            finished_at: &'a str,
            duration_ms: i64,
            error: &'a str,
        }

        fs::create_dir_all(directory).with_context(|| {
            format!(
                "failed to create terminal journal directory {}",
                directory.display()
            )
        })?;
        if !terminals.is_empty() {
            let path = directory.join("shutdown-terminal-quarantine.jsonl");
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| {
                    format!("failed to open terminal recovery sink {}", path.display())
                })?;
            for terminal in terminals {
                serde_json::to_writer(
                    &mut file,
                    &ShutdownTerminalRecoveryLine {
                        invoke_id: terminal.record.invoke_id.clone(),
                        occurred_at: terminal.record.occurred_at.clone(),
                        raw_capture: terminal.raw_capture,
                        acknowledged: false,
                        error: Some(error.to_string()),
                        record: Some(terminal.record.clone()),
                    },
                )
                .context("failed to encode terminal recovery entry")?;
                file.write_all(b"\n")
                    .context("failed to append terminal recovery delimiter")?;
            }
            file.sync_data()
                .context("failed to sync terminal recovery sink")?;
        }
        if !finishes.is_empty() {
            let path = directory.join("shutdown-system-task-quarantine.jsonl");
            let mut file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .with_context(|| {
                    format!(
                        "failed to open system-task recovery sink {}",
                        path.display()
                    )
                })?;
            for finish in finishes {
                serde_json::to_writer(
                    &mut file,
                    &SystemTaskEntry {
                        run_id: finish.run_id,
                        task_kind: finish.task_kind.as_str(),
                        trigger_kind: &finish.trigger_kind,
                        status: finish.status.as_str(),
                        summary: &finish.summary,
                        detail: &finish.detail,
                        finished_at: &finish.finished_at,
                        duration_ms: finish.duration_ms,
                        error,
                    },
                )
                .context("failed to encode system-task recovery entry")?;
                file.write_all(b"\n")
                    .context("failed to append system-task recovery delimiter")?;
            }
            file.sync_data()
                .context("failed to sync system-task recovery sink")?;
        }
        Ok(())
    }

    pub(crate) fn open(database_path: &Path) -> Result<Self> {
        let directory = terminal_journal_directory(database_path);
        fs::create_dir_all(&directory).with_context(|| {
            format!(
                "failed to create terminal journal directory {}",
                directory.display()
            )
        })?;

        let (mut segments, loaded_segments, acknowledged_sequences, next_sequence, pending_bytes) =
            load_terminal_journal_segments(&directory)?;
        let current_start = segments
            .last_key_value()
            .map(|(_, segment)| segment.first_sequence)
            .unwrap_or(next_sequence);
        let mut pending_by_key = HashMap::new();
        let mut replay_segments = VecDeque::new();
        let mut replay_count = 0_usize;
        for (first_sequence, entries) in loaded_segments {
            let mut segment_has_replay = false;
            for entry in entries {
                if acknowledged_sequences.contains(&entry.sequence) {
                    continue;
                }
                pending_by_key
                    .entry((entry.invoke_id, entry.occurred_at, entry.raw_capture))
                    .or_insert_with(Vec::new)
                    .push(entry.sequence);
                replay_count = replay_count.saturating_add(1);
                segment_has_replay = true;
            }
            if segment_has_replay {
                replay_segments.push_back(JournalReplayCursor {
                    first_sequence,
                    byte_offset: 0,
                });
            }
        }
        segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists")
            .current = true;

        let current_path = segments
            .get(&current_start)
            .expect("current terminal journal segment exists")
            .path
            .clone();
        let shutdown_recovery_path = directory.join("shutdown-terminal-quarantine.jsonl");
        let (shutdown_recovery_pending, recovery_writes) =
            load_shutdown_terminal_recovery(&shutdown_recovery_path);
        let mut deferred_writes = VecDeque::new();
        for terminal in recovery_writes {
            let key = (
                terminal.record.invoke_id.clone(),
                terminal.record.occurred_at.clone(),
                terminal.raw_capture,
            );
            if pending_by_key.contains_key(&key) {
                continue;
            }
            pending_by_key.insert(key, Vec::new());
            deferred_writes.push_back(terminal);
        }
        let system_task_quarantine_ids = load_system_task_quarantine_ids(&[
            directory.join("system-task-quarantine.jsonl"),
            directory.join("shutdown-system-task-quarantine.jsonl"),
        ]);
        let deferred_system_task_finishes = load_system_task_finishes(&[
            directory.join("system-task-quarantine.jsonl"),
            directory.join("shutdown-system-task-quarantine.jsonl"),
        ]);
        let replayed_system_task_ids = deferred_system_task_finishes
            .iter()
            .map(|finish| finish.run_id)
            .collect();
        let journal = Self {
            directory,
            current_file: open_append_file(&current_path)?,
            segments,
            pending_by_key,
            next_sequence,
            pending_bytes,
            replay_segments,
            replay_count: replay_count.saturating_add(deferred_writes.len()),
            deferred_writes,
            replay_blocked: false,
            shutdown_recovery_pending,
            system_task_quarantine_ids,
            replayed_system_task_ids,
            deferred_system_task_finishes,
            last_sync_at: Instant::now(),
            last_checkpoint_at: Instant::now(),
            overflowed: pending_bytes >= TERMINAL_JOURNAL_MAX_BYTES,
            sync_failed: false,
        };
        Ok(journal)
    }

    pub(crate) fn append(
        &mut self,
        record: &ProxyCaptureRecord,
        raw_capture: bool,
        capture_started: Option<Instant>,
    ) -> TerminalJournalAppendOutcome {
        let key = (
            record.invoke_id.clone(),
            record.occurred_at.clone(),
            raw_capture,
        );
        if self.sync_failed {
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if self.pending_by_key.contains_key(&key) {
            return self.append_outcome(TerminalJournalDurabilityMode::Journal, None);
        }
        let entry = JournalTerminalRecord {
            sequence: self.next_sequence,
            raw_capture,
            capture_elapsed_ms: capture_started
                .map(|started| started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64),
            record: record.clone(),
        };
        let Ok(encoded) = encode_entry_line(&entry) else {
            warn!(invoke_id = %record.invoke_id, occurred_at = %record.occurred_at, "terminal journal serialization failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        };
        let encoded_len = encoded.len() as u64;
        if self.pending_bytes.saturating_add(encoded_len) > TERMINAL_JOURNAL_MAX_BYTES {
            self.overflowed = true;
            warn!(
                journal_pending_bytes = self.pending_bytes,
                journal_max_bytes = TERMINAL_JOURNAL_MAX_BYTES,
                invoke_id = %record.invoke_id,
                occurred_at = %record.occurred_at,
                "terminal journal reached capacity; using memory fallback"
            );
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.rotate_if_needed(encoded_len) {
            warn!(error = %err, "terminal journal rotation failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        if let Err(err) = self.current_file.write_all(&encoded) {
            warn!(error = %err, "terminal journal append failed; using memory fallback");
            self.overflowed = true;
            return self.append_outcome(TerminalJournalDurabilityMode::MemoryOverflow, None);
        }
        let current_start = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current terminal journal segment exists");
        let segment = self
            .segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists");
        segment.last_sequence = entry.sequence;
        segment.bytes = segment.bytes.saturating_add(encoded_len);
        segment.sequences.insert(entry.sequence);
        self.pending_by_key.insert(key, vec![entry.sequence]);
        self.next_sequence = self.next_sequence.saturating_add(1);
        self.pending_bytes = self.pending_bytes.saturating_add(encoded_len);
        self.append_outcome(TerminalJournalDurabilityMode::Journal, Some(entry.sequence))
    }

    pub(crate) fn sync_if_due(&mut self) -> Option<u64> {
        if self.last_sync_at.elapsed() < TERMINAL_JOURNAL_SYNC_INTERVAL {
            return None;
        }
        let started = Instant::now();
        match self.current_file.sync_data() {
            Ok(()) => {
                self.last_sync_at = Instant::now();
                self.sync_failed = false;
                Some(started.elapsed().as_millis() as u64)
            }
            Err(err) => {
                warn!(error = %err, "terminal journal group commit failed");
                self.overflowed = true;
                self.sync_failed = true;
                None
            }
        }
    }

    pub(crate) fn force_sync(&mut self) -> Result<()> {
        match self.current_file.sync_data() {
            Ok(()) => {
                self.last_sync_at = Instant::now();
                self.sync_failed = false;
                Ok(())
            }
            Err(err) => {
                self.overflowed = true;
                self.sync_failed = true;
                Err(err).context("failed to sync terminal journal")
            }
        }
    }

    pub(crate) fn acknowledge(&mut self, invoke_id: &str, occurred_at: &str, raw_capture: bool) {
        let key = (invoke_id.to_string(), occurred_at.to_string(), raw_capture);
        let recovery_pending = self.shutdown_recovery_pending.contains_key(&key);
        let Some(sequences) = self
            .pending_by_key
            .get(&key)
            .cloned()
            .or_else(|| recovery_pending.then_some(Vec::new()))
        else {
            return;
        };
        let recovery_only = recovery_pending && sequences.is_empty();
        let mut recovery_ack_durable = true;
        if recovery_pending {
            if let Err(err) = append_shutdown_terminal_ack(
                &self.directory.join("shutdown-terminal-quarantine.jsonl"),
                invoke_id,
                occurred_at,
                raw_capture,
            ) {
                warn!(error = %err, invoke_id, occurred_at, "failed to checkpoint shutdown terminal recovery acknowledgement");
                self.sync_failed = true;
                self.overflowed = true;
                recovery_ack_durable = false;
                if recovery_only {
                    return;
                }
            } else {
                self.shutdown_recovery_pending.remove(&key);
            }
        }
        let (pending_sequences, acknowledged_replay_count) = self.write_acknowledgements(sequences);
        if pending_sequences.is_empty() {
            self.pending_by_key.remove(&key);
        } else {
            self.pending_by_key.insert(key, pending_sequences);
        }
        self.last_checkpoint_at = Instant::now();
        self.replay_count = self
            .replay_count
            .saturating_sub(acknowledged_replay_count + usize::from(recovery_only));
        self.remove_fully_acknowledged_segments();
        if recovery_pending
            && recovery_ack_durable
            && let Err(err) = compact_shutdown_terminal_recovery(
                &self.directory.join("shutdown-terminal-quarantine.jsonl"),
                &self.shutdown_recovery_pending,
            )
        {
            warn!(error = %err, "failed to compact shutdown terminal recovery sink");
        }
        if !self.sync_failed && self.pending_bytes < TERMINAL_JOURNAL_MAX_BYTES * 3 / 5 {
            self.overflowed = false;
        }
    }

    fn write_acknowledgements(&mut self, sequences: Vec<u64>) -> (Vec<u64>, usize) {
        let mut pending_sequences = Vec::new();
        let mut written_sequences = Vec::new();
        for sequence in sequences {
            let Ok(encoded) = encode_acknowledgement_line(sequence) else {
                warn!(
                    sequence,
                    "terminal journal acknowledgement serialization failed"
                );
                pending_sequences.push(sequence);
                continue;
            };
            if let Err(err) = self.current_file.write_all(&encoded) {
                warn!(error = %err, sequence, "terminal journal acknowledgement append failed");
                pending_sequences.push(sequence);
                continue;
            }
            let current_start = self
                .segments
                .iter()
                .find_map(|(start, segment)| segment.current.then_some(*start))
                .expect("current terminal journal segment exists");
            let segment = self
                .segments
                .get_mut(&current_start)
                .expect("current terminal journal segment exists");
            segment.bytes = segment.bytes.saturating_add(encoded.len() as u64);
            self.pending_bytes = self.pending_bytes.saturating_add(encoded.len() as u64);
            written_sequences.push(sequence);
        }
        if written_sequences.is_empty() {
            return (pending_sequences, 0);
        }
        if let Err(err) = self.current_file.sync_data() {
            warn!(
                error = %err,
                count = written_sequences.len(),
                "terminal journal acknowledgement checkpoint failed"
            );
            self.sync_failed = true;
            self.overflowed = true;
            pending_sequences.extend(written_sequences);
            return (pending_sequences, 0);
        }
        self.last_sync_at = Instant::now();
        for sequence in &written_sequences {
            if let Some(start) = self
                .segments
                .iter()
                .find_map(|(start, segment)| segment.sequences.contains(sequence).then_some(*start))
            {
                self.segments
                    .get_mut(&start)
                    .expect("terminal journal acknowledgement target exists")
                    .acknowledged
                    .insert(*sequence);
            }
        }
        (pending_sequences, written_sequences.len())
    }

    pub(crate) fn remember_shutdown_recovery(
        &mut self,
        terminals: &[&BatchedTerminalInvocationWrite],
    ) {
        for terminal in terminals {
            let key = (
                terminal.record.invoke_id.clone(),
                terminal.record.occurred_at.clone(),
                terminal.raw_capture,
            );
            if self
                .shutdown_recovery_pending
                .insert(key.clone(), terminal.record.clone())
                .is_none()
                && !self.pending_by_key.contains_key(&key)
            {
                self.replay_count = self.replay_count.saturating_add(1);
            }
        }
    }

    pub(crate) fn take_replay(&mut self) -> Vec<BatchedTerminalInvocationWrite> {
        self.take_replay_chunk(usize::MAX, usize::MAX)
    }

    pub(crate) fn queue_replay_for_dispatch(&mut self, max_writes: usize) {
        if max_writes == 0 || self.deferred_writes.len() >= max_writes {
            return;
        }
        let available = max_writes.saturating_sub(self.deferred_writes.len());
        let replay = self.take_replay_chunk(available, TERMINAL_JOURNAL_REPLAY_MAX_BYTES);
        self.deferred_writes.extend(replay);
    }

    pub(crate) fn defer_write(&mut self, write: BatchedTerminalInvocationWrite) -> bool {
        self.deferred_writes.push_back(write);
        true
    }

    pub(crate) fn deferred_write_count(&self) -> usize {
        self.deferred_writes.len()
    }

    pub(crate) fn take_deferred_writes(
        &mut self,
        max_writes: usize,
    ) -> Vec<BatchedTerminalInvocationWrite> {
        let count = max_writes.min(self.deferred_writes.len());
        self.deferred_writes.drain(..count).collect()
    }

    pub(crate) fn stats(&self) -> TerminalJournalStats {
        TerminalJournalStats {
            pending_records: self.pending_by_key.len(),
            pending_bytes: self.pending_bytes,
            segment_count: self.segments.len(),
            replay_count: self.replay_count,
            checkpoint_lag_ms: self.last_checkpoint_at.elapsed().as_millis() as u64,
            overflowed: self.overflowed,
        }
    }

    fn append_outcome(
        &self,
        durability_mode: TerminalJournalDurabilityMode,
        sequence: Option<u64>,
    ) -> TerminalJournalAppendOutcome {
        let stats = self.stats();
        TerminalJournalAppendOutcome {
            durability_mode,
            sequence,
            pending_records: stats.pending_records,
            pending_bytes: stats.pending_bytes,
        }
    }

    fn rotate_if_needed(&mut self, next_len: u64) -> Result<()> {
        let current_start = self
            .segments
            .iter()
            .find_map(|(start, segment)| segment.current.then_some(*start))
            .expect("current terminal journal segment exists");
        let should_rotate = self.segments.get(&current_start).is_some_and(|segment| {
            segment.bytes > 0
                && segment.bytes.saturating_add(next_len) > TERMINAL_JOURNAL_SEGMENT_BYTES
        });
        if !should_rotate {
            return Ok(());
        }
        self.current_file
            .sync_data()
            .context("failed to sync terminal journal before rotation")?;
        let start = self.next_sequence;
        let path = segment_path(&self.directory, start);
        let next_file = open_append_file(&path)?;
        self.segments
            .get_mut(&current_start)
            .expect("current terminal journal segment exists")
            .current = false;
        self.current_file = next_file;
        self.segments.insert(
            start,
            JournalSegment {
                path,
                first_sequence: start,
                last_sequence: start.saturating_sub(1),
                bytes: 0,
                sequences: HashSet::new(),
                acknowledged: HashSet::new(),
                current: true,
            },
        );
        self.remove_fully_acknowledged_segments();
        Ok(())
    }

    fn remove_fully_acknowledged_segments(&mut self) {
        let removable = self
            .segments
            .iter()
            .filter_map(|(start, segment)| {
                (!segment.current && segment.is_fully_acknowledged()).then_some(*start)
            })
            .collect::<Vec<_>>();
        for start in removable {
            if let Some(segment) = self.segments.remove(&start) {
                if let Err(err) = fs::remove_file(&segment.path) {
                    warn!(error = %err, path = %segment.path.display(), "failed to remove acknowledged terminal journal segment");
                    self.segments.insert(start, segment);
                } else {
                    self.pending_bytes = self.pending_bytes.saturating_sub(segment.bytes);
                }
            }
        }
    }
}

fn load_terminal_journal_segments(
    directory: &Path,
) -> Result<(
    BTreeMap<u64, JournalSegment>,
    Vec<(u64, Vec<JournalEntryMetadata>)>,
    HashSet<u64>,
    u64,
    u64,
)> {
    let mut paths = fs::read_dir(directory)
        .with_context(|| {
            format!(
                "failed to list terminal journal directory {}",
                directory.display()
            )
        })?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "jsonl")
                && path
                    .file_stem()
                    .and_then(|stem| stem.to_str())
                    .is_some_and(|stem| stem.starts_with("terminal-"))
        })
        .collect::<Vec<_>>();
    paths.sort();
    let mut segments = BTreeMap::new();
    let mut loaded_segments = Vec::new();
    let mut acknowledged_sequences = HashSet::new();
    let mut next_sequence = 1_u64;
    let mut pending_bytes = 0_u64;
    for path in paths {
        let loaded = load_segment(&path)?;
        next_sequence = next_sequence.max(loaded.segment.last_sequence.saturating_add(1));
        pending_bytes = pending_bytes.saturating_add(loaded.segment.bytes);
        acknowledged_sequences.extend(loaded.acknowledgement_sequences);
        loaded_segments.push((loaded.segment.first_sequence, loaded.entries));
        segments.insert(loaded.segment.first_sequence, loaded.segment);
    }
    let current_start = segments
        .last_key_value()
        .map(|(_, segment)| segment.first_sequence)
        .unwrap_or(next_sequence);
    segments
        .entry(current_start)
        .or_insert_with(|| JournalSegment {
            path: segment_path(directory, current_start),
            first_sequence: current_start,
            last_sequence: current_start.saturating_sub(1),
            bytes: 0,
            sequences: HashSet::new(),
            acknowledged: HashSet::new(),
            current: true,
        });
    for segment in segments.values_mut() {
        segment.acknowledged.extend(
            acknowledged_sequences
                .intersection(&segment.sequences)
                .copied(),
        );
    }
    Ok((
        segments,
        loaded_segments,
        acknowledged_sequences,
        next_sequence,
        pending_bytes,
    ))
}
