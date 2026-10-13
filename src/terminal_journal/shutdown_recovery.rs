use super::*;

pub(super) fn load_shutdown_terminal_recovery(path: &Path) -> LoadedShutdownTerminalRecovery {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                warn!(path = %path.display(), %error, "shutdown terminal recovery unavailable; retaining source and continuing startup");
            }
            return (
                HashMap::new(),
                Vec::new(),
                error.kind() == io::ErrorKind::NotFound,
                Vec::new(),
            );
        }
    };
    let metadata = match file.metadata() {
        Ok(metadata)
            if metadata.is_file()
                && metadata.len() <= TERMINAL_JOURNAL_MAX_DEFERRED_BYTES as u64 =>
        {
            metadata
        }
        result => {
            warn!(path = %path.display(), metadata = ?result, "shutdown terminal recovery source unreadable or oversized; retaining source and continuing startup");
            return (HashMap::new(), Vec::new(), false, Vec::new());
        }
    };
    let mut reader = BufReader::new(file.take(metadata.len()));
    let mut line = Vec::new();
    let mut entries = HashMap::<ShutdownRecoveryKey, Option<ShutdownRecoveryValue>>::new();
    let mut orders = HashMap::<ShutdownRecoveryKey, u64>::new();
    let mut damaged = false;
    let mut readable = true;
    let mut line_number = 0_u64;
    loop {
        line.clear();
        let read = match reader.read_until(b'\n', &mut line) {
            Ok(read) => read,
            Err(error) => {
                warn!(path = %path.display(), %error, line_number, "shutdown terminal recovery read failed; retaining source and continuing startup");
                readable = false;
                break;
            }
        };
        if read == 0 {
            break;
        }
        line_number += 1;
        let entry = match serde_json::from_slice::<ShutdownTerminalRecoveryLine>(&line) {
            Ok(entry) if entry.acknowledged || entry.record.is_some() => entry,
            _ => {
                if !line.ends_with(b"\n") {
                    warn!(path = %path.display(), line_number, ignored_bytes = line.len(), "ignoring incomplete shutdown terminal recovery tail");
                    break;
                }
                damaged = true;
                warn!(path = %path.display(), line_number, "isolating damaged shutdown terminal recovery frame");
                continue;
            }
        };
        let key = (entry.invoke_id, entry.occurred_at, entry.raw_capture);
        orders.insert(key.clone(), line_number);
        if entry.acknowledged {
            entries.insert(key, None);
        } else if let Some(record) = entry.record {
            entries.insert(key, Some((record, entry.attempts)));
        }
    }
    let mut ordered_keys = entries
        .iter()
        .filter_map(|(key, value)| value.as_ref().map(|_| key.clone()))
        .collect::<Vec<_>>();
    ordered_keys.sort_by_key(|key| orders.get(key).copied().unwrap_or_default());
    let pending = entries
        .into_iter()
        .filter_map(|(key, value)| value.map(|value| (key, value)))
        .collect::<HashMap<_, _>>();
    let writes = ordered_keys
        .iter()
        .filter_map(|key| pending.get(key).map(|value| (key, value)))
        .map(
            |((invoke_id, occurred_at, raw_capture), (record, attempts))| {
                BatchedTerminalInvocationWrite {
                    enqueued_at: None,
                    diagnostic: None,
                    capture_started: None,
                    raw_capture: *raw_capture,
                    attempts: attempts.clone(),
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
                }
            },
        )
        .collect();
    let ordered_pending = ordered_keys
        .iter()
        .filter_map(|key| pending.get(key).map(|value| (key.clone(), value.clone())))
        .collect::<Vec<_>>();
    // Never replace a partially unread source, or damaged bytes before isolation is durable.
    let clean_source = readable
        && (!damaged || isolate_damaged_source(path).is_ok())
        && match compact_shutdown_terminal_recovery(path, &ordered_pending) {
            Ok(()) => true,
            Err(error) => {
                warn!(path = %path.display(), %error, "shutdown terminal recovery cleanup failed; retaining source");
                false
            }
        };
    (pending, writes, clean_source, ordered_keys)
}

fn isolate_damaged_source(path: &Path) -> Result<()> {
    let isolated = path.with_extension(format!("jsonl.corrupt-{}", nanoid::nanoid!()));
    let result = (|| -> Result<()> {
        let mut original = File::open(path)?;
        let mut backup = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&isolated)?;
        io::copy(&mut original, &mut backup)?;
        backup.sync_all()?;
        if let Some(parent) = path.parent() {
            File::open(parent)?.sync_all()?;
        }
        Ok(())
    })();
    match &result {
        Ok(()) => {
            warn!(path = %path.display(), isolated_path = %isolated.display(), "preserved damaged shutdown terminal recovery source")
        }
        Err(error) => {
            warn!(path = %path.display(), %error, "shutdown terminal recovery isolation failed; retaining original source")
        }
    }
    result
}

pub(super) fn open_shutdown_recovery_append(path: &Path, create: bool) -> Result<File> {
    let mut file = OpenOptions::new()
        .create(create)
        .read(true)
        .append(true)
        .open(path)?;
    if file.metadata()?.len() > 0 {
        file.seek(SeekFrom::End(-1))?;
        let mut last_byte = [0];
        file.read_exact(&mut last_byte)?;
        if last_byte[0] != b'\n' {
            // A failed startup cleanup must not concatenate the next fact/ACK with a torn tail.
            warn!(path = %path.display(), "delimiting retained shutdown terminal recovery tail before append");
            file.write_all(b"\n")?;
        }
    }
    Ok(file)
}

pub(super) fn compact_shutdown_terminal_recovery(
    path: &Path,
    pending: &[(ShutdownRecoveryKey, ShutdownRecoveryValue)],
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
    for ((invoke_id, occurred_at, raw_capture), (record, attempts)) in pending {
        serde_json::to_writer(
            &mut file,
            &ShutdownTerminalRecoveryLine {
                invoke_id: invoke_id.clone(),
                occurred_at: occurred_at.clone(),
                raw_capture: *raw_capture,
                acknowledged: false,
                error: Some("recovery snapshot".to_string()),
                record: Some(record.clone()),
                attempts: attempts.clone(),
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
    if let Some(parent) = path.parent() {
        File::open(parent)?.sync_all()?;
    }
    Ok(())
}
