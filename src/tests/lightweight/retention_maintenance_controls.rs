use super::*;

#[tokio::test]
async fn ownership_manual_materialization_deferral_does_not_schedule_automatic_work() {
    let scheduler = StartupBackfillScheduler::default();
    let task = StartupBackfillTask::PromptCacheConversationsMaterialization;
    let original_due = Utc::now() + ChronoDuration::hours(1);
    scheduler.record_next_due(task, original_due);
    let retry_at = Utc::now() + ChronoDuration::seconds(60);
    let gate = crate::db_pressure::DbPressureGate::new(1, Duration::from_secs(30));
    crate::maintenance::with_maintenance_execution_options(
        crate::maintenance::MaintenanceExecutionOptions {
            manual: true,
            admitted: true,
            dry_run: false,
        },
        async {
            assert!(
                startup_backfill_pressure_defer_outcome_at(
                    &scheduler,
                    task,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
                    retry_at,
                )
                .is_pressure_deferred()
            );
            assert!(
                startup_backfill_coordinator_defer_outcome(&scheduler, task, &gate)
                    .is_pressure_deferred()
            );
            assert!(
                startup_backfill_coordinator_defer_outcome_at(
                    &scheduler,
                    task,
                    crate::db_pressure::DbPressureDenyReason::BackgroundBusy,
                    retry_at,
                )
                .is_pressure_deferred()
            );
        },
    )
    .await;
    assert_eq!(
        scheduler.next_due.lock().unwrap().get(&task),
        Some(&original_due)
    );
    assert_eq!(scheduler.health_snapshot().pressure_defer_count, 0);
}

#[tokio::test]
async fn ownership_admitted_materialization_round_survives_a_later_trigger_pause() {
    use crate::maintenance_store::{
        PromptCacheMaterializationControl, PromptCacheMaterializationStepAdmission,
    };
    let global = Arc::new(PromptCacheMaterializationControl::default());
    let snapshot = global.initialize(true);
    global.publish_committed(false);
    // Admission keeps the captured snapshot even when pause commits before it is copied.
    let admitted = PromptCacheMaterializationControl::admitted_control(snapshot);
    assert!(
        admitted
            .lock_current_generation(snapshot.generation)
            .await
            .is_some()
    );
    assert!(matches!(
        admitted.begin_step(Some(snapshot.generation)),
        PromptCacheMaterializationStepAdmission::Started(_)
    ));
    assert!(matches!(
        global.begin_step(None),
        PromptCacheMaterializationStepAdmission::Disabled
    ));
}
