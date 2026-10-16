use super::*;

#[test]
fn performance_index_cli_requires_positive_budget_and_preserves_retention_flag_exclusion() {
    let cli = CliArgs::try_parse_from([
        "cvm",
        "maintenance",
        "prepare-performance-indexes",
        "--dry-run",
    ])
    .expect("optional index command");
    let Some(CliCommand::Maintenance(MaintenanceCliArgs {
        command: MaintenanceCommand::PreparePerformanceIndexes(options),
    })) = &cli.command
    else {
        panic!("index preparation selected")
    };
    assert!(options.dry_run);
    assert_eq!(options.budget_ms, 2_000);
    assert!(
        selected_owned_maintenance(&cli).unwrap().is_none(),
        "index command uses its own ownership-aware route"
    );
    assert!(
        CliArgs::try_parse_from([
            "cvm",
            "maintenance",
            "prepare-performance-indexes",
            "--budget-ms",
            "0"
        ])
        .is_err()
    );
    let combined = CliArgs::try_parse_from([
        "cvm",
        "--retention-run-once",
        "maintenance",
        "prepare-performance-indexes",
    ])
    .expect("parsed before semantic exclusion");
    assert!(selected_owned_maintenance(&combined).is_err());
}
