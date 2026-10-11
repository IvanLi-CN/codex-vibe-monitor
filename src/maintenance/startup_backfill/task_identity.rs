use super::{StartupBackfillTask, SystemTaskKind};

pub(super) fn observation_parent_task_key(
    selected_tasks: Option<&[StartupBackfillTask]>,
) -> Option<&'static str> {
    selected_tasks
        .filter(|tasks| {
            tasks.len() == 1
                && tasks[0] == StartupBackfillTask::PromptCacheConversationsMaterialization
        })
        .map(|_| "prompt_cache_materialization")
}

pub(super) fn audit_task_kind(selected_tasks: Option<&[StartupBackfillTask]>) -> SystemTaskKind {
    if selected_tasks.is_some_and(|tasks| {
        tasks.len() == 1 && tasks[0] == StartupBackfillTask::PromptCacheConversationsMaterialization
    }) {
        SystemTaskKind::PromptCacheMaterialization
    } else {
        SystemTaskKind::StartupBackfill
    }
}

pub(super) fn should_record_audit(
    managed_run_id: Option<i64>,
    ran_actionable_task: bool,
    had_failure: bool,
    has_detail: bool,
    cancelled: bool,
) -> bool {
    managed_run_id.is_none() && (ran_actionable_task || had_failure || has_detail) && !cancelled
}
