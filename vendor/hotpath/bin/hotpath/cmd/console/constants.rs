//! Constants used across the TUI application

use std::sync::LazyLock;

pub(crate) static SAMPLY_LOAD_DISABLED: LazyLock<bool> =
    LazyLock::new(|| hotpath::env_flag("HOTPATH_DISABLE_SAMPLY_LOAD"));
