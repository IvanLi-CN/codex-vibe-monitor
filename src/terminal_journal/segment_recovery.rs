use super::*;
use serde_json::value::RawValue;

#[derive(Deserialize)]
struct RawJournalLine<'a> {
    event_version: Option<u64>,
    #[serde(borrow)]
    entry: Option<&'a RawValue>,
    #[serde(borrow)]
    attempt_start: Option<&'a RawValue>,
    acknowledged_sequence: Option<u64>,
    checksum: &'a str,
}

fn verified_frame(raw: &[u8]) -> std::result::Result<JournalLine, &'static str> {
    let line: RawJournalLine<'_> = serde_json::from_slice(raw).map_err(|_| "invalid_json")?;
    let version = line
        .event_version
        .unwrap_or(TERMINAL_JOURNAL_EVENT_VERSION.into());
    if version != u64::from(TERMINAL_JOURNAL_EVENT_VERSION) {
        return Err("unsupported_version");
    }
    if let Some(start) = line.attempt_start {
        #[derive(Deserialize)]
        struct StartVersion {
            event_version: Option<u64>,
        }
        let start: StartVersion =
            serde_json::from_str(start.get()).map_err(|_| "invalid_record")?;
        if start
            .event_version
            .unwrap_or(TERMINAL_JOURNAL_EVENT_VERSION.into())
            != u64::from(TERMINAL_JOURNAL_EVENT_VERSION)
        {
            return Err("unsupported_version");
        }
    }
    let encoded = serde_json::to_vec(&(
        version,
        line.entry,
        line.attempt_start,
        line.acknowledged_sequence,
    ))
    .map_err(|_| "checksum_encoding")?;
    let mut checksum_matches = format!("{:x}", Sha256::digest(encoded)) == line.checksum;
    if !checksum_matches && line.event_version.is_none() && line.attempt_start.is_none() {
        let legacy = serde_json::to_vec(&(line.entry, line.acknowledged_sequence))
            .map_err(|_| "checksum_encoding")?;
        checksum_matches = format!("{:x}", Sha256::digest(legacy)) == line.checksum;
    }
    if !checksum_matches {
        return Err("checksum_mismatch");
    }
    // Business decoding happens only after verification of the original JSON values.
    // Re-encoding typed floats is not a stable checksum input for legacy records.
    let parsed: JournalLine = serde_json::from_slice(raw).map_err(|_| "invalid_record")?;
    match (
        &parsed.entry,
        &parsed.attempt_start,
        parsed.acknowledged_sequence,
    ) {
        (Some(_), None, None) | (None, Some(_), None) | (None, None, Some(_)) => Ok(parsed),
        _ => Err("invalid_event_shape"),
    }
}

pub(super) fn load_segment(path: &Path) -> Result<LoadedJournalSegment> {
    let raw = fs::read(path)
        .with_context(|| format!("failed to read terminal journal segment {}", path.display()))?;
    let mut entries = Vec::new();
    let mut attempt_starts = Vec::new();
    let mut acknowledged = HashSet::new();
    let mut valid_frames = Vec::with_capacity(raw.len());
    let mut damaged = false;
    for (line_number, raw_line) in raw.split_inclusive(|byte| *byte == b'\n').enumerate() {
        let line_number = line_number + 1;
        if !raw_line.ends_with(b"\n") {
            damaged = true;
            warn!(path = %path.display(), line_number, ignored_bytes = raw_line.len(), "ignoring incomplete terminal journal tail");
            break;
        }
        if raw_line.iter().all(u8::is_ascii_whitespace) {
            valid_frames.extend_from_slice(raw_line);
            continue;
        }
        let line = match verified_frame(raw_line) {
            Ok(line) => line,
            Err("unsupported_version") => {
                anyhow::bail!(
                    "unsupported terminal journal event version in {} at line {}; segment preserved",
                    path.display(),
                    line_number
                );
            }
            Err(reason) => {
                damaged = true;
                warn!(path = %path.display(), line_number, reason, "isolating damaged terminal journal frame");
                continue;
            }
        };
        match (line.entry, line.attempt_start, line.acknowledged_sequence) {
            (Some(entry), None, None) => entries.push(JournalEntryMetadata {
                sequence: entry.sequence,
                invoke_id: entry.record.invoke_id,
                occurred_at: entry.record.occurred_at,
                raw_capture: entry.raw_capture,
                attempt_public_ids: entry
                    .attempts
                    .into_iter()
                    .map(|attempt| attempt.attempt_public_id)
                    .collect(),
            }),
            (None, Some(start), None) => attempt_starts.push(JournalAttemptStartMetadata {
                sequence: start.sequence,
                snapshot: start.attempt,
            }),
            (None, None, Some(sequence)) => {
                acknowledged.insert(sequence);
            }
            _ => unreachable!("verified event shape"),
        }
        valid_frames.extend_from_slice(raw_line);
    }
    if damaged {
        preserve_and_replace_segment(path, &raw, &valid_frames)?;
    }
    let first_sequence = terminal_journal_segment_start(path)?;
    let sequences = entries
        .iter()
        .map(|entry| entry.sequence)
        .chain(attempt_starts.iter().map(|entry| entry.sequence))
        .collect::<HashSet<_>>();
    // A retained ACK can outlive an isolated event. Never reuse its sequence.
    let last_sequence = sequences
        .iter()
        .chain(acknowledged.iter())
        .copied()
        .max()
        .unwrap_or(first_sequence.saturating_sub(1));
    Ok(LoadedJournalSegment {
        segment: JournalSegment {
            path: path.to_path_buf(),
            first_sequence,
            last_sequence,
            bytes: valid_frames.len() as u64,
            sequences,
            acknowledged: HashSet::new(),
            current: false,
        },
        entries,
        attempt_starts,
        acknowledgement_sequences: acknowledged,
    })
}

fn private_new_file(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn preserve_and_replace_segment(path: &Path, original: &[u8], cleaned: &[u8]) -> Result<()> {
    let evidence = path.with_extension(format!("jsonl.corrupt-{}", nanoid::nanoid!()));
    let temporary = path.with_extension(format!("jsonl.recovery-{}.tmp", nanoid::nanoid!()));
    let parent = path
        .parent()
        .context("terminal journal segment has no directory")?;
    let directory = File::open(parent)?;
    // Copy first: a crash must leave either the original active source or the
    // complete cleaned source visible to the next startup, never only a backup.
    let mut backup = private_new_file(&evidence)
        .context("failed to preserve damaged terminal journal source")?;
    backup.write_all(original)?;
    backup.sync_all()?;
    directory.sync_all()?;
    warn!(path = %path.display(), evidence_path = %evidence.display(), "preserved damaged terminal journal source");
    let result = (|| -> Result<()> {
        let mut file = private_new_file(&temporary)?;
        file.write_all(cleaned)?;
        file.sync_all()?;
        fs::rename(&temporary, path)?;
        directory.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        // Only our uniquely named unpublished temporary file is removable.
        let _ = fs::remove_file(&temporary);
    }
    result.with_context(|| {
        format!(
            "failed to publish recovered terminal segment {}",
            path.display()
        )
    })
}
