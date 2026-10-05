use super::*;

#[cfg(unix)]
struct TaskWorkOwnerChild(std::process::Child);

#[cfg(unix)]
impl Drop for TaskWorkOwnerChild {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[cfg(unix)]
#[test]
#[ignore = "child process for the archive work ownership regression"]
fn retention_task_work_owner_child_fixture() {
    use std::os::fd::AsRawFd;
    let path = PathBuf::from(std::env::var_os("CVM_TEST_TASK_WORK_FILE").expect("child work path"));
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .expect("open child work file");
    assert_eq!(unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) }, 0);
    fs::write(path.with_extension("ready"), b"locked").expect("child readiness");
    std::thread::sleep(Duration::from_secs(60));
    drop(file);
}

#[cfg(unix)]
#[tokio::test]
async fn retention_task_work_cleanup_survives_reused_pid_and_owner_process_death() {
    use std::os::fd::AsRawFd;
    let (pool, config, temp_dir) =
        retention_test_pool_and_config("task-work-owner-process-death").await;
    seed_task_work_source(&pool, &config, 0).await;
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    let month = shanghai_month_key_from_local_naive(&occurred_at).expect("month");
    let target = archive_batch_file_path(&config, "codex_invocations", &month).expect("target");
    fs::create_dir_all(target.parent().expect("archive parent")).expect("archive directory");
    // Both filenames claim the current, live PID. Only the kernel lock identifies the
    // active owner, so a restarted container's reuse of PID 1 cannot preserve old work.
    let abandoned = PathBuf::from(format!(
        "{}.task-{}-1-0.sqlite",
        target.display(),
        std::process::id()
    ));
    let active = PathBuf::from(format!(
        "{}.task-{}-2-0.sqlite",
        target.display(),
        std::process::id()
    ));
    let abandoned_gzip = abandoned.with_extension("tmp");
    let active_wal = PathBuf::from(format!("{}-wal", active.display()));
    let unrelated = PathBuf::from(format!("{}.task-12-unrelated.sqlite", target.display()));
    for path in [
        &abandoned,
        &abandoned_gzip,
        &active,
        &active_wal,
        &unrelated,
    ] {
        fs::write(path, b"task fixture; never resume these contents").expect("work fixture");
    }
    let mut child = TaskWorkOwnerChild(
        std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args([
                "--exact",
                "tests::archive_file_io::retention_task_work_ownership::retention_task_work_owner_child_fixture",
                "--ignored",
            ])
            .env("CVM_TEST_TASK_WORK_FILE", &active)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn owned work process"),
    );
    let ready = active.with_extension("ready");
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("child holds file lock");
    let first = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("first task");
    assert_eq!(first.0, 1);
    assert!(
        !abandoned.exists(),
        "reused live PID must not keep abandoned work"
    );
    assert!(!abandoned_gzip.exists());
    assert!(
        active.exists(),
        "live kernel owner must retain its work file"
    );
    assert!(active_wal.exists());
    assert!(
        unrelated.exists(),
        "malformed names are outside cleanup ownership"
    );

    child.0.kill().expect("kill only owned child");
    child.0.wait().expect("reap owned child");
    seed_task_work_source(&pool, &config, 1).await;
    // Work-file creation must not block behind another archive directory owner.
    let directory = fs::File::open(target.parent().expect("parent")).expect("directory fence");
    assert_eq!(
        unsafe { libc::flock(directory.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM codex_invocations")
        .fetch_all(&pool)
        .await
        .expect("remaining source");
    let refused = archive_rows_into_month_batch(
        &pool,
        &config,
        archive_table_spec("codex_invocations"),
        &month,
        &ids,
    )
    .await
    .expect_err("busy directory refuses new work");
    assert_eq!(
        refused.to_string(),
        "retention write deferred before archive_work_cleanup"
    );
    assert!(
        active.exists(),
        "no cleanup while another directory owner holds the fence"
    );
    drop(directory);
    let second = archive_old_invocations(&pool, &config, config.database_path.parent(), false)
        .await
        .expect("fresh task after process death");
    assert_eq!(second.0, 1);
    assert!(!active.exists());
    assert!(!active_wal.exists());
    assert!(unrelated.exists());
    fs::remove_file(ready).expect("remove child marker");
    fs::remove_file(unrelated).expect("remove explicit unrelated fixture");
    pool.close().await;
    cleanup_temp_test_dir(&temp_dir);
}

#[cfg(unix)]
async fn seed_task_work_source(pool: &SqlitePool, config: &AppConfig, index: i64) {
    let occurred_at = shanghai_local_days_ago((config.invocation_max_days + 2) as i64, 12, 0, 0);
    sqlx::query("INSERT INTO codex_invocations(invoke_id,occurred_at,source,status,total_tokens,cost,payload,raw_response) VALUES(?1,?2,'proxy','success',7,0.07,'{}','{}')")
        .bind(format!("work-owner-{index}"))
        .bind(occurred_at)
        .execute(pool)
        .await
        .expect("fresh live source");
}
