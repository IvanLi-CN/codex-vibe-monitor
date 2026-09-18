use std::{
    collections::{BTreeMap, HashMap, HashSet, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::warn;

use crate::{
    BatchedSystemTaskFinish, BatchedTerminalInvocationWrite, ProxyCaptureRecord, SystemTaskKind,
    SystemTaskStatus, api_invocation_from_runtime_record, startup_backfill_tasks_for_terminal,
};

include!("terminal_journal/part_01.rs");
include!("terminal_journal/part_02.rs");
include!("terminal_journal/part_03.rs");
include!("terminal_journal/part_04.rs");
