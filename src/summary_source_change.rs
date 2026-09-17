//! Durable, compact source-change descriptors used to recover Summary projections.
//!
//! The journal deliberately stores identity and reconstruction metadata only. Raw request or
//! response text, previews, and duplicate Summary rows never enter this table. Terminal source
//! writes append a descriptor through the same SQLite transaction as the source row; readers can
//! therefore treat a committed descriptor as an exact durable tail.

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use futures_util::TryStreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Pool, Row, Sqlite, SqliteConnection};
use std::io::Read;

pub(crate) const SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_ENTRIES: usize = 10_000;
pub(crate) const SUMMARY_SOURCE_CHANGE_JOURNAL_MAX_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const SUMMARY_SOURCE_CHANGE_DESCRIPTOR_VERSION: i64 = 1;
pub(crate) const SUMMARY_SOURCE_CHANGE_CHECKPOINT_SCOPE: &str = "summary-global";
pub(crate) const SUMMARY_ARCHIVE_SNAPSHOT_MAX_RECORDS: usize = 400;
const SUMMARY_ARCHIVE_SNAPSHOT_MAX_PROOF_PAGES: i64 = 4_096;
const SUMMARY_ARCHIVE_SNAPSHOT_MAX_PROOF_PAYLOAD_BYTES: i64 = 512 * 1024 * 1024;
pub(crate) const SUMMARY_ARCHIVE_SNAPSHOT_V1: i64 = 1;
pub(crate) const SUMMARY_ARCHIVE_SNAPSHOT_V2: i64 = 2;

include!("summary_source_change/part-01.rs");
include!("summary_source_change/part-02.rs");
include!("summary_source_change/part-03.rs");
include!("summary_source_change/part-04.rs");
include!("summary_source_change/part-05.rs");
