use super::*;

#[test]
fn managed_task_sse_topics_have_stable_bounded_contracts() {
    let cases = [
        (
            "system.managed-tasks.runtime",
            "system.managed-tasks.runtime/v1",
        ),
        (
            "system.managed-tasks.timeline",
            "system.managed-tasks.timeline/v1",
        ),
        (
            "system.managed-tasks.catalog",
            "system.managed-tasks.catalog/v1",
        ),
    ];
    for (name, epoch) in cases {
        let descriptor = SubscriptionTopicDescriptor {
            topic: name.to_string(),
            params: BTreeMap::new(),
        };
        let topic = SubscriptionTopic::from_descriptor(&descriptor)
            .expect("managed task topic should parse");
        assert_eq!(topic.descriptor(), descriptor);
        assert_eq!(topic.name(), name);
        assert_eq!(topic.class(), SubscriptionTopicClass::BoundedColdHydrate);
        assert_eq!(topic.schema_epoch(), epoch);
        assert!(topic.runtime_topic_dependencies().is_empty());
    }

    let timeline_v2_descriptor = SubscriptionTopicDescriptor {
        topic: "system.managed-tasks.timeline".to_string(),
        params: btree_map_from_pairs([("schemaVersion", "2".to_string())]),
    };
    let timeline_v2 = SubscriptionTopic::from_descriptor(&timeline_v2_descriptor)
        .expect("versioned timeline topic should parse");
    assert_eq!(timeline_v2.descriptor(), timeline_v2_descriptor);
    assert_eq!(
        timeline_v2.schema_epoch(),
        "system.managed-tasks.timeline/v2"
    );
    assert_ne!(
        SubscriptionTopic::ManagedTaskTimeline
            .cache_key()
            .expect("legacy timeline topic cache key"),
        timeline_v2
            .cache_key()
            .expect("versioned timeline topic cache key")
    );
    assert_eq!(
        timeline_v2.class(),
        SubscriptionTopicClass::BoundedColdHydrate
    );
    assert!(timeline_v2.runtime_topic_dependencies().is_empty());
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "system.managed-tasks.timeline".to_string(),
            params: btree_map_from_pairs([("schemaVersion", "3".to_string())]),
        })
        .is_err()
    );

    let detail_descriptor = SubscriptionTopicDescriptor {
        topic: "system.managed-tasks.detail".to_string(),
        params: btree_map_from_pairs([("taskKey", "retention_archive".to_string())]),
    };
    let detail = SubscriptionTopic::from_descriptor(&detail_descriptor)
        .expect("managed task detail topic should parse");
    assert_eq!(detail.descriptor(), detail_descriptor);
    assert_eq!(detail.schema_epoch(), "system.managed-tasks.detail/v1");
    assert_eq!(detail.class(), SubscriptionTopicClass::BoundedColdHydrate);
    assert!(detail.runtime_topic_dependencies().is_empty());
    assert!(
        SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
            topic: "system.managed-tasks.detail".to_string(),
            params: BTreeMap::new(),
        })
        .is_err()
    );

    let workload_descriptor = SubscriptionTopicDescriptor {
        topic: "system.managed-tasks.workload".to_string(),
        params: btree_map_from_pairs([
            ("taskKey", "retention_archive".to_string()),
            ("windowHours", "24".to_string()),
            ("limit", "200".to_string()),
        ]),
    };
    let workload = SubscriptionTopic::from_descriptor(&workload_descriptor)
        .expect("managed task workload topic should parse");
    assert_eq!(workload.descriptor(), workload_descriptor);
    assert_eq!(workload.schema_epoch(), "system.managed-tasks.workload/v1");
    assert_eq!(workload.class(), SubscriptionTopicClass::BoundedColdHydrate);
    assert!(workload.runtime_topic_dependencies().is_empty());
    for params in [
        btree_map_from_pairs([
            ("taskKey", "retention_archive".to_string()),
            ("windowHours", "23".to_string()),
            ("limit", "200".to_string()),
        ]),
        btree_map_from_pairs([
            ("taskKey", "retention_archive".to_string()),
            ("windowHours", "24".to_string()),
            ("limit", "201".to_string()),
        ]),
        btree_map_from_pairs([
            ("windowHours", "24".to_string()),
            ("limit", "200".to_string()),
        ]),
    ] {
        assert!(
            SubscriptionTopic::from_descriptor(&SubscriptionTopicDescriptor {
                topic: "system.managed-tasks.workload".to_string(),
                params,
            })
            .is_err()
        );
    }
}

#[tokio::test]
async fn managed_task_timeline_transport_pages_more_than_ten_thousand_rows() {
    const SEGMENT_COUNT: usize = 13_120;
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect timeline transport fixture");
    sqlx::query("CREATE TABLE maintenance_metadata (key TEXT PRIMARY KEY,value TEXT NOT NULL,updated_at TEXT NOT NULL)")
        .execute(&pool)
        .await
        .expect("create timeline metadata fixture");
    sqlx::query("INSERT INTO maintenance_metadata (key,value,updated_at) VALUES ('task_timeline_revision',?,?)")
        .bind(SEGMENT_COUNT.to_string())
        .bind("2026-10-04T00:00:00.000Z")
        .execute(&pool)
        .await
        .expect("seed timeline revision");
    sqlx::query("CREATE TABLE task_timeline_segments (segment_id TEXT PRIMARY KEY,kind TEXT NOT NULL,task_key TEXT NOT NULL,title TEXT NOT NULL,started_at TEXT NOT NULL,last_observed_at TEXT NOT NULL,finished_at TEXT,duration_ms INTEGER,status TEXT NOT NULL,trigger_kind TEXT,execution_class TEXT,reason TEXT,retry_at TEXT,active_child_task_key TEXT,active_child_title TEXT,managed_run_id INTEGER,session_id TEXT NOT NULL,revision INTEGER NOT NULL)")
        .execute(&pool)
        .await
        .expect("create timeline segment fixture");
    sqlx::query("CREATE TABLE task_timeline_coverage (session_id TEXT PRIMARY KEY,started_at TEXT NOT NULL,last_seen_at TEXT NOT NULL,ended_at TEXT,dropped_events INTEGER NOT NULL DEFAULT 0)")
        .execute(&pool)
        .await
        .expect("create timeline coverage fixture");

    let now = Utc::now();
    let observed_at = format_utc_iso_millis(now - ChronoDuration::minutes(1));
    let mut transaction = pool.begin().await.expect("begin timeline fixture");
    for revision in 1..=SEGMENT_COUNT {
        sqlx::query("INSERT INTO task_timeline_segments (segment_id,kind,task_key,title,started_at,last_observed_at,status,trigger_kind,session_id,revision) VALUES (?, 'execution','retention_archive','Retention archive',?,?, 'success','interval','fixture-session',?)")
            .bind(format!("fixture-{revision:05}"))
            .bind(&observed_at)
            .bind(&observed_at)
            .bind(revision as i64)
            .execute(&mut *transaction)
            .await
            .expect("insert timeline fixture segment");
    }
    transaction.commit().await.expect("commit timeline fixture");

    let store = crate::maintenance_store::MaintenanceStore::from_pool(pool);
    assert!(
        managed_task_timeline_page_payload(&store, None)
            .await
            .is_err()
    );
    let payload = managed_task_timeline_revision_payload(&store)
        .await
        .expect("timeline revision notification should not aggregate segment rows");
    assert_eq!(
        payload.get("watermark").and_then(Value::as_i64),
        Some(SEGMENT_COUNT as i64)
    );
    assert!(payload.get("observedAt").and_then(Value::as_str).is_some());
    assert_eq!(payload.as_object().map(serde_json::Map::len), Some(2));

    let from = format_utc_iso_millis(now - ChronoDuration::hours(12));
    let to = format_utc_iso_millis(now);
    let mut cursor = None;
    let mut loaded = 0;
    let mut page_count = 0;
    let mut baseline_ids = HashSet::new();
    loop {
        let page = crate::task_timeline::timeline_page(
            &store,
            cursor.as_deref(),
            None,
            Some(&from),
            Some(&to),
            None,
            500,
        )
        .await
        .expect("read fixed-watermark timeline page");
        assert_eq!(page.watermark, SEGMENT_COUNT as i64);
        assert_eq!(page.window_start, from);
        assert_eq!(page.window_end, to);
        loaded += page.segments.len();
        baseline_ids.extend(
            page.segments
                .iter()
                .map(|segment| segment.segment_id.clone()),
        );
        page_count += 1;
        cursor = page.next_cursor;
        if page_count == 1 {
            let mut mutation = store.pool.begin().await.expect("begin page mutation");
            sqlx::query(
                "UPDATE maintenance_metadata SET value=? WHERE key='task_timeline_revision'",
            )
            .bind((SEGMENT_COUNT + 1).to_string())
            .execute(&mut *mutation)
            .await
            .expect("advance revision between baseline pages");
            sqlx::query(
                "UPDATE task_timeline_segments SET revision=? WHERE segment_id='fixture-00001'",
            )
            .bind((SEGMENT_COUNT + 1) as i64)
            .execute(&mut *mutation)
            .await
            .expect("revise an earlier baseline row between pages");
            mutation.commit().await.expect("commit page mutation");
        }
        if cursor.is_none() {
            break;
        }
    }
    assert_eq!(loaded, SEGMENT_COUNT);
    assert_eq!(page_count, 27);
    assert_eq!(baseline_ids.len(), SEGMENT_COUNT);
    assert!(baseline_ids.contains("fixture-00501"));

    sqlx::query("UPDATE maintenance_metadata SET value=? WHERE key='task_timeline_revision'")
        .bind((SEGMENT_COUNT + 2).to_string())
        .execute(&store.pool)
        .await
        .expect("advance timeline revision");
    sqlx::query("INSERT INTO task_timeline_segments (segment_id,kind,task_key,title,started_at,last_observed_at,status,trigger_kind,session_id,revision) VALUES ('fixture-delta','execution','retention_archive','Retention archive',?,?, 'success','interval','fixture-session',?)")
        .bind(&observed_at)
        .bind(&observed_at)
        .bind((SEGMENT_COUNT + 2) as i64)
        .execute(&store.pool)
        .await
        .expect("insert timeline revision delta");
    let delta = crate::task_timeline::timeline_page(
        &store,
        None,
        Some(SEGMENT_COUNT as i64),
        Some(&from),
        Some(&to),
        None,
        500,
    )
    .await
    .expect("read revision delta after the complete baseline");
    assert_eq!(delta.watermark, (SEGMENT_COUNT + 2) as i64);
    assert_eq!(delta.segments.len(), 2);
    let delta_ids = delta
        .segments
        .iter()
        .map(|segment| segment.segment_id.as_str())
        .collect::<HashSet<_>>();
    assert_eq!(delta_ids, HashSet::from(["fixture-00001", "fixture-delta"]));
    assert!(delta.next_cursor.is_none());
}
