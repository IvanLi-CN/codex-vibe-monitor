use std::{
    collections::VecDeque,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use aes_gcm::{
    Aes256Gcm, KeyInit,
    aead::{Aead, Payload},
};
use anyhow::{Context, Result, bail};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::RuntimeRouteEvent;

const MAX_JOURNAL_BYTES: usize = 64 * 1024 * 1024;
const MAX_EVENT_BYTES: usize = 128 * 1024;
const COMPACTION_BYTES: usize = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct Frame {
    version: u8,
    sequence: u64,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}

// This sidecar shares the terminal journal directory but has its own checkpoint.
// Recovery stores routing metadata only. Conversation identifiers are encrypted,
// and no raw error body, credentials, request or response payload enters an event.
pub(super) struct RouteJournal {
    path: PathBuf,
    checkpoint_path: PathBuf,
    file: File,
    key: [u8; 32],
    bytes: usize,
    next_sequence: u64,
    frames: VecDeque<(u64, Vec<u8>)>,
    append_failed: bool,
}

impl std::fmt::Debug for RouteJournal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RouteJournal")
            .field("bytes", &self.bytes)
            .field("pending_frames", &self.frames.len())
            .finish_non_exhaustive()
    }
}

impl RouteJournal {
    pub(super) fn open(
        directory: &Path,
        seed: &[u8; 32],
    ) -> Result<(Self, Vec<RuntimeRouteEvent>)> {
        fs::create_dir_all(directory).context("create route recovery directory")?;
        let path = directory.join("proxy-route-facts-v1.events");
        let checkpoint_path = directory.join("proxy-route-facts-v1.checkpoint");
        let mut digest = Sha256::new();
        digest.update(b"codex-vibe-monitor/proxy-route-facts/v1\0");
        digest.update(seed);
        let key: [u8; 32] = digest.finalize().into();
        let acknowledged = if checkpoint_path.exists() {
            let bytes = fs::read(&checkpoint_path).context("read route checkpoint")?;
            let frame: Frame = serde_json::from_slice(&bytes).context("decode route checkpoint")?;
            let checkpoint: u64 = serde_json::from_slice(&decode(&key, &frame, b"checkpoint")?)?;
            if checkpoint != frame.sequence {
                bail!("route checkpoint sequence mismatch");
            }
            checkpoint
        } else {
            0
        };
        let raw = match fs::read(&path) {
            Ok(raw) => raw,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(error) => return Err(error).context("read route journal"),
        };
        if raw.len() > MAX_JOURNAL_BYTES {
            bail!("route recovery journal exceeds hard capacity");
        }
        let mut events = Vec::new();
        let mut frames = VecDeque::new();
        let mut next_sequence = acknowledged.saturating_add(1);
        let mut previous_sequence = 0;
        // Fail closed on damage or a future format without rewriting accepted facts.
        for raw_line in raw.split_inclusive(|byte| *byte == b'\n') {
            if !raw_line.ends_with(b"\n") {
                bail!("incomplete route journal frame; original preserved");
            }
            if raw_line.len() > MAX_EVENT_BYTES {
                bail!("route journal frame exceeds recovery bound");
            }
            let frame: Frame = serde_json::from_slice(raw_line)
                .context("decode route journal frame; original preserved")?;
            if frame.sequence <= previous_sequence {
                bail!("non-monotonic route journal sequence");
            }
            previous_sequence = frame.sequence;
            let event: RuntimeRouteEvent =
                serde_json::from_slice(&decode(&key, &frame, b"event")?)?;
            if event.sequence != frame.sequence {
                bail!("route event sequence mismatch");
            }
            next_sequence = next_sequence.max(frame.sequence.saturating_add(1));
            if frame.sequence > acknowledged {
                events.push(event);
                frames.push_back((frame.sequence, raw_line.to_vec()));
            }
        }
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("open route journal append")?;
        File::open(directory)?
            .sync_all()
            .context("sync route journal directory")?;
        Ok((
            Self {
                path,
                checkpoint_path,
                file,
                key,
                bytes: raw.len(),
                next_sequence,
                frames,
                append_failed: false,
            },
            events,
        ))
    }

    pub(super) fn append(&mut self, event: &mut RuntimeRouteEvent) -> Result<()> {
        if self.append_failed {
            bail!("route journal has an incomplete append; restart recovery required");
        }
        let next_sequence = self
            .next_sequence
            .checked_add(1)
            .context("route sequence exhausted")?;
        event.sequence = self.next_sequence;
        let encoded = encode(
            &self.key,
            event.sequence,
            b"event",
            &serde_json::to_vec(event)?,
        )?;
        if encoded.len() > MAX_EVENT_BYTES {
            bail!("route event exceeds hard frame capacity");
        }
        if self.bytes.saturating_add(encoded.len()) > MAX_JOURNAL_BYTES {
            self.compact()?;
            if self.bytes.saturating_add(encoded.len()) > MAX_JOURNAL_BYTES {
                bail!("route journal capacity exhausted; observation not accepted");
            }
        }
        if let Err(error) = self.file.write_all(&encoded) {
            self.append_failed = true;
            return Err(error).context("append route event");
        }
        self.bytes += encoded.len();
        self.frames.push_back((event.sequence, encoded));
        self.next_sequence = next_sequence;
        Ok(())
    }

    pub(super) fn sync(&mut self) -> Result<()> {
        if self.append_failed {
            bail!("route journal append failed; no durability acknowledgement");
        }
        self.file
            .sync_data()
            .context("group-sync route observations")
    }

    pub(super) fn append_failed(&self) -> bool {
        self.append_failed
    }

    pub(super) fn acknowledge(&mut self, sequence: u64) -> Result<()> {
        let encoded = encode(
            &self.key,
            sequence,
            b"checkpoint",
            &serde_json::to_vec(&sequence)?,
        )?;
        atomic_replace(&self.checkpoint_path, &encoded)?;
        while self
            .frames
            .front()
            .is_some_and(|(candidate, _)| *candidate <= sequence)
        {
            self.frames.pop_front();
        }
        let pending_bytes: usize = self.frames.iter().map(|(_, frame)| frame.len()).sum();
        if self.bytes >= COMPACTION_BYTES && pending_bytes < self.bytes / 2 {
            self.compact()?;
        }
        Ok(())
    }

    fn compact(&mut self) -> Result<()> {
        let mut pending = Vec::new();
        for (_, frame) in &self.frames {
            pending.extend_from_slice(frame);
        }
        atomic_replace(&self.path, &pending)?;
        self.file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .context("reopen compacted route journal")?;
        self.bytes = pending.len();
        Ok(())
    }
}

fn encode(key: &[u8; 32], sequence: u64, kind: &[u8], plaintext: &[u8]) -> Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid route recovery key"))?;
    let mut nonce = [0_u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce);
    let mut aad = kind.to_vec();
    aad.extend_from_slice(&sequence.to_le_bytes());
    aad.push(1);
    let ciphertext = cipher
        .encrypt(
            aes_gcm::Nonce::from_slice(&nonce),
            Payload {
                msg: plaintext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("encrypt route recovery metadata"))?;
    let mut bytes = serde_json::to_vec(&Frame {
        version: 1,
        sequence,
        nonce: nonce.to_vec(),
        ciphertext,
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn decode(key: &[u8; 32], frame: &Frame, kind: &[u8]) -> Result<Vec<u8>> {
    if frame.version != 1 {
        bail!(
            "unsupported route recovery version {}; original preserved",
            frame.version
        );
    }
    if frame.nonce.len() != 12 {
        bail!("invalid route journal nonce length");
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| anyhow::anyhow!("invalid route recovery key"))?;
    let mut aad = kind.to_vec();
    aad.extend_from_slice(&frame.sequence.to_le_bytes());
    aad.push(frame.version);
    cipher
        .decrypt(
            aes_gcm::Nonce::from_slice(&frame.nonce),
            Payload {
                msg: &frame.ciphertext,
                aad: &aad,
            },
        )
        .map_err(|_| anyhow::anyhow!("route journal authentication failed; original preserved"))
}

fn atomic_replace(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    File::open(
        path.parent()
            .context("route journal has no parent directory")?,
    )?
    .sync_all()?;
    Ok(())
}
