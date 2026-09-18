struct ReplayBudgetReader<'a> {
    reader: &'a mut BufReader<File>,
    remaining: usize,
}

impl Read for ReplayBudgetReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal journal replay record exceeds the bounded input budget",
            ));
        }
        let limit = buffer.len().min(self.remaining);
        let read = self.reader.read(&mut buffer[..limit])?;
        self.remaining = self.remaining.saturating_sub(read);
        Ok(read)
    }
}

fn append_shutdown_terminal_ack(
    path: &Path,
    invoke_id: &str,
    occurred_at: &str,
    raw_capture: bool,
) -> Result<()> {
    if !path.exists() {
        return Err(anyhow::anyhow!(
            "shutdown terminal recovery file is missing: {}",
            path.display()
        ));
    }
    let mut file = OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| {
            format!(
                "failed to open shutdown terminal recovery {}",
                path.display()
            )
        })?;
    serde_json::to_writer(
        &mut file,
        &ShutdownTerminalRecoveryLine {
            invoke_id: invoke_id.to_string(),
            occurred_at: occurred_at.to_string(),
            raw_capture,
            acknowledged: true,
            error: None,
            record: None,
        },
    )
    .context("failed to encode shutdown terminal recovery acknowledgement")?;
    file.write_all(b"\n")
        .context("failed to append shutdown terminal recovery acknowledgement")?;
    file.sync_data()
        .context("failed to sync shutdown terminal recovery acknowledgement")?;
    Ok(())
}

fn load_system_task_quarantine_ids(paths: &[PathBuf]) -> HashSet<i64> {
    let mut ids = HashSet::new();
    for path in paths {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            if read == 0 {
                break;
            }
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line)
                && let Some(run_id) = value.get("run_id").and_then(serde_json::Value::as_i64)
            {
                ids.insert(run_id);
            }
        }
    }
    ids
}

fn append_system_task_acknowledgements(path: &Path, run_ids: &[i64]) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .with_context(|| format!("failed to open system-task quarantine {}", path.display()))?;
    for run_id in run_ids {
        serde_json::to_writer(
            &mut file,
            &serde_json::json!({ "run_id": run_id, "acknowledged": true }),
        )
        .context("failed to encode system-task quarantine acknowledgement")?;
        file.write_all(b"\n")
            .context("failed to append system-task quarantine acknowledgement")?;
    }
    file.sync_data()
        .context("failed to sync system-task quarantine acknowledgement")?;
    Ok(())
}

fn load_system_task_finishes(paths: &[PathBuf]) -> VecDeque<BatchedSystemTaskFinish> {
    let mut entries = HashMap::<i64, (Option<BatchedSystemTaskFinish>, bool)>::new();
    for path in paths {
        let Ok(file) = File::open(path) else {
            continue;
        };
        let mut reader = BufReader::new(file);
        let mut line = String::new();
        loop {
            line.clear();
            let Ok(read) = reader.read_line(&mut line) else {
                break;
            };
            if read == 0 {
                break;
            }
            let Ok(value) = serde_json::from_str::<SystemTaskRecoveryLine>(&line) else {
                continue;
            };
            if value.acknowledged {
                entries.insert(value.run_id, (None, true));
                continue;
            }
            let (Some(task_kind), Some(status)) = (
                parse_system_task_kind(&value.task_kind),
                parse_system_task_status(&value.status),
            ) else {
                continue;
            };
            if entries
                .get(&value.run_id)
                .is_some_and(|(_, acknowledged)| *acknowledged)
            {
                continue;
            }
            entries.insert(
                value.run_id,
                (
                    Some(BatchedSystemTaskFinish {
                        run_id: value.run_id,
                        task_kind,
                        trigger_kind: value.trigger_kind,
                        status,
                        summary: value.summary,
                        detail: value.detail,
                        finished_at: value.finished_at,
                        duration_ms: value.duration_ms,
                    }),
                    false,
                ),
            );
        }
    }
    entries
        .into_values()
        .filter_map(|(finish, acknowledged)| (!acknowledged).then_some(finish).flatten())
        .collect()
}

fn parse_system_task_kind(value: &str) -> Option<SystemTaskKind> {
    match value {
        "retention_archive" => Some(SystemTaskKind::RetentionArchive),
        "startup_backfill" => Some(SystemTaskKind::StartupBackfill),
        "hourly_rollup_bootstrap" => Some(SystemTaskKind::HourlyRollupBootstrap),
        "forward_proxy_subscription_refresh" => {
            Some(SystemTaskKind::ForwardProxySubscriptionRefresh)
        }
        _ => None,
    }
}

fn parse_system_task_status(value: &str) -> Option<SystemTaskStatus> {
    match value {
        "running" => Some(SystemTaskStatus::Running),
        "success" => Some(SystemTaskStatus::Success),
        "failed" => Some(SystemTaskStatus::Failed),
        "skipped" => Some(SystemTaskStatus::Skipped),
        _ => None,
    }
}

fn load_shutdown_terminal_recovery(path: &Path) -> LoadedShutdownTerminalRecovery {
    let Ok(file) = File::open(path) else {
        return (HashMap::new(), Vec::new());
    };
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut entries = HashMap::<ShutdownRecoveryKey, Option<ProxyCaptureRecord>>::new();
    let mut corrupt = false;
    loop {
        line.clear();
        let Ok(read) = reader.read_line(&mut line) else {
            corrupt = true;
            break;
        };
        if read == 0 {
            break;
        }
        let Ok(entry) = serde_json::from_str::<ShutdownTerminalRecoveryLine>(&line) else {
            warn!(path = %path.display(), "ignoring corrupt shutdown terminal recovery line");
            corrupt = true;
            continue;
        };
        let key = (entry.invoke_id, entry.occurred_at, entry.raw_capture);
        if entry.acknowledged {
            entries.insert(key, None);
        } else if entry.record.is_some() {
            entries.insert(key, entry.record);
        }
    }
    let pending = entries
        .into_iter()
        .filter_map(|((invoke_id, occurred_at, raw_capture), record)| {
            let record = record?;
            Some(((invoke_id, occurred_at, raw_capture), record))
        })
        .collect::<HashMap<_, _>>();
    let writes = pending
        .iter()
        .map(
            |((invoke_id, occurred_at, raw_capture), record)| BatchedTerminalInvocationWrite {
                capture_started: None,
                raw_capture: *raw_capture,
                // Shutdown recovery has the same post-restart sequence boundary as journal
                // replay and must not force Summary into a full rolling rebuild.
                dashboard_terminal_sequence: Some(0),
                terminal_projection_event_ids: Vec::new(),
                startup_backfill_tasks: startup_backfill_tasks_for_terminal(
                    &api_invocation_from_runtime_record(record),
                ),
                record: ProxyCaptureRecord {
                    invoke_id: invoke_id.clone(),
                    occurred_at: occurred_at.clone(),
                    ..record.clone()
                },
            },
        )
        .collect();
    if !corrupt {
        let _ = compact_shutdown_terminal_recovery(path, &pending);
    }
    (pending, writes)
}

fn compact_shutdown_terminal_recovery(
    path: &Path,
    pending: &HashMap<ShutdownRecoveryKey, ProxyCaptureRecord>,
) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let temporary_path = path.with_extension("jsonl.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary_path)
        .with_context(|| {
            format!(
                "failed to open temporary shutdown terminal recovery {}",
                temporary_path.display()
            )
        })?;
    for ((invoke_id, occurred_at, raw_capture), record) in pending {
        serde_json::to_writer(
            &mut file,
            &ShutdownTerminalRecoveryLine {
                invoke_id: invoke_id.clone(),
                occurred_at: occurred_at.clone(),
                raw_capture: *raw_capture,
                acknowledged: false,
                error: Some("recovery snapshot".to_string()),
                record: Some(record.clone()),
            },
        )
        .context("failed to encode shutdown terminal recovery snapshot")?;
        file.write_all(b"\n")
            .context("failed to append shutdown terminal recovery snapshot delimiter")?;
    }
    file.sync_data()
        .context("failed to sync shutdown terminal recovery snapshot")?;
    fs::rename(&temporary_path, path).with_context(|| {
        format!(
            "failed to atomically publish shutdown terminal recovery {}",
            path.display()
        )
    })?;
    Ok(())
}

fn segment_path(directory: &Path, first_sequence: u64) -> PathBuf {
    directory.join(format!("terminal-{first_sequence:020}.jsonl"))
}

fn terminal_journal_directory(database_path: &Path) -> PathBuf {
    let parent = database_path.parent().unwrap_or_else(|| Path::new("."));
    let database_id = format!(
        "{:x}",
        Sha256::digest(database_path.as_os_str().as_encoded_bytes())
    );
    parent.join(format!("terminal_journal-{database_id}"))
}

fn open_append_file(path: &Path) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .with_context(|| format!("failed to open terminal journal segment {}", path.display()))
}

fn load_segment(path: &Path) -> Result<LoadedJournalSegment> {
    let raw = fs::read(path)
        .with_context(|| format!("failed to read terminal journal segment {}", path.display()))?;
    let mut entries = Vec::new();
    let mut acknowledged = HashSet::new();
    let mut valid_bytes = 0usize;
    let mut corrupt_tail = false;
    for (line_number, raw_line) in raw.split_inclusive(|byte| *byte == b'\n').enumerate() {
        if !raw_line.ends_with(b"\n") {
            corrupt_tail = true;
            break;
        }
        let line = match std::str::from_utf8(&raw_line[..raw_line.len() - 1]) {
            Ok(line) => line,
            Err(_) => {
                corrupt_tail = true;
                break;
            }
        };
        if line.trim().is_empty() {
            valid_bytes += raw_line.len();
            continue;
        }
        let parsed = serde_json::from_str::<JournalLine>(line);
        let Ok(line) = parsed else {
            warn!(path = %path.display(), line_number, "terminal journal has corrupt line; repairing segment tail");
            corrupt_tail = true;
            break;
        };
        if journal_line_checksum(line.entry.as_ref(), line.acknowledged_sequence)? != line.checksum
        {
            warn!(path = %path.display(), line_number, "terminal journal checksum mismatch; repairing segment tail");
            corrupt_tail = true;
            break;
        }
        match (line.entry, line.acknowledged_sequence) {
            (Some(entry), None) => entries.push(JournalEntryMetadata {
                sequence: entry.sequence,
                invoke_id: entry.record.invoke_id,
                occurred_at: entry.record.occurred_at,
                raw_capture: entry.raw_capture,
            }),
            (None, Some(sequence)) => {
                acknowledged.insert(sequence);
            }
            _ => {
                warn!(path = %path.display(), line_number, "terminal journal line has invalid event shape; repairing segment tail");
                corrupt_tail = true;
                break;
            }
        }
        valid_bytes += raw_line.len();
    }
    if corrupt_tail {
        repair_corrupt_segment_tail(path, &raw[..valid_bytes])?;
    }
    let first_sequence = terminal_journal_segment_start(path)?;
    let last_sequence = entries
        .last()
        .map(|entry| entry.sequence)
        .unwrap_or(first_sequence.saturating_sub(1));
    let sequences = entries
        .iter()
        .map(|entry| entry.sequence)
        .collect::<HashSet<_>>();
    Ok(LoadedJournalSegment {
        segment: JournalSegment {
            path: path.to_path_buf(),
            first_sequence,
            last_sequence,
            bytes: valid_bytes as u64,
            sequences,
            acknowledged: HashSet::new(),
            current: false,
        },
        entries,
        acknowledgement_sequences: acknowledged,
    })
}

fn terminal_journal_segment_start(path: &Path) -> Result<u64> {
    path.file_stem()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_prefix("terminal-"))
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "invalid terminal journal segment filename: {}",
                path.display()
            )
        })
}

fn encode_entry_line(entry: &JournalTerminalRecord) -> Result<Vec<u8>> {
    let line = JournalLine {
        entry: Some(entry.clone()),
        acknowledged_sequence: None,
        checksum: journal_line_checksum(Some(entry), None)?,
    };
    let mut bytes = serde_json::to_vec(&line).context("failed to encode terminal journal line")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn encode_acknowledgement_line(sequence: u64) -> Result<Vec<u8>> {
    let line = JournalLine {
        entry: None,
        acknowledged_sequence: Some(sequence),
        checksum: journal_line_checksum(None, Some(sequence))?,
    };
    let mut bytes =
        serde_json::to_vec(&line).context("failed to encode terminal journal acknowledgement")?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn journal_line_checksum(
    entry: Option<&JournalTerminalRecord>,
    acknowledged_sequence: Option<u64>,
) -> Result<String> {
    let encoded = serde_json::to_vec(&(entry, acknowledged_sequence))
        .context("failed to checksum terminal journal event")?;
    Ok(format!("{:x}", Sha256::digest(encoded)))
}

fn repair_corrupt_segment_tail(path: &Path, valid_prefix: &[u8]) -> Result<()> {
    let evidence_path = path.with_extension(format!(
        "jsonl.corrupt-{}",
        chrono::Utc::now().timestamp_millis()
    ));
    fs::rename(path, &evidence_path).with_context(|| {
        format!(
            "failed to preserve corrupt terminal journal segment {}",
            path.display()
        )
    })?;
    fs::write(path, valid_prefix).with_context(|| {
        format!(
            "failed to rebuild terminal journal segment {} after corruption",
            path.display()
        )
    })?;
    warn!(
        path = %path.display(),
        evidence_path = %evidence_path.display(),
        valid_prefix_bytes = valid_prefix.len(),
        "repaired terminal journal corrupt tail while preserving evidence"
    );
    Ok(())
}
