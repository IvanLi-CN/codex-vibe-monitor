impl SubscriptionHub {
    async fn apply_dashboard_activity_live_overlay(
        &self,
        state: Arc<AppState>,
        topic: &SubscriptionTopic,
        live: DashboardActivityLiveSnapshot,
    ) -> Result<(), ApiError> {
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            if !apply_dashboard_activity_live_overlay_to_payload(
                state.as_ref(),
                &mut payload,
                &live,
            )? {
                return Ok(());
            }

            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serde_json::to_vec(&payload)?,
            )?);
            let payload_bytes = frame.payload_bytes.len();
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload.clone();
            cached.snapshot_frame = frame.clone();
            cached.snapshot_bytes = payload_bytes;
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: Utc::now(),
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);

            SubscriptionDispatchEvent { frame }
        };

        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }

    async fn apply_dashboard_network_slice_to_timeseries(
        &self,
        topic: &SubscriptionTopic,
        slice: &DashboardNetworkProjectionSlice,
    ) -> Result<(), ApiError> {
        let SubscriptionTopic::DashboardNetworkTimeseriesWindow {
            upstream_account_id,
            ..
        } = topic
        else {
            return Ok(());
        };
        let bucket = match upstream_account_id {
            None => slice.network_live_bucket.clone(),
            Some(upstream_account_id) => slice
                .accounts
                .iter()
                .find(|account| account.upstream_account_id == Some(*upstream_account_id))
                .and_then(|account| account.network_live_bucket.clone()),
        };
        let Some(bucket) = bucket else {
            return Ok(());
        };
        let topic_key = topic.cache_key()?;
        self.dashboard_topology_counters
            .record_json_overlay(topic.name());
        let dispatch = {
            let mut guard = self.state.lock().await;
            let Some(cached) = guard.topics.get_mut(&topic_key) else {
                return Ok(());
            };
            let mut payload = cached.snapshot_payload.clone();
            let Some(object) = payload.as_object_mut() else {
                return Ok(());
            };
            let bucket_value = serde_json::to_value(&bucket)?;
            let bucket_start = bucket_value.get("bucketStart").cloned();
            let Some(points) = object.get_mut("points").and_then(Value::as_array_mut) else {
                return Ok(());
            };
            let point_index = points
                .iter()
                .position(|point| point.get("bucketStart") == bucket_start.as_ref())
                .or_else(|| {
                    points.iter().position(|point| {
                        point
                            .get("isLiveBucket")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    })
                });
            let Some(point_index) = point_index else {
                return Ok(());
            };
            let now = Utc::now();
            points[point_index] = bucket_value;
            object.insert(
                "rangeEnd".to_string(),
                Value::String(format_utc_iso_precise(now)),
            );
            object.insert(
                "snapshotId".to_string(),
                Value::from(now.timestamp_millis()),
            );
            let serialized = serde_json::to_vec(&payload)?;
            if cached.snapshot_frame.payload_bytes.as_ref() == serialized.as_slice() {
                return Ok(());
            }
            let next_cursor = cached.cursor.saturating_add(1);
            let frame = Arc::new(self.serialize_frame(
                cached.descriptor.clone(),
                topic_key.clone(),
                cached.schema_epoch.clone(),
                next_cursor,
                serialized,
            )?);
            let retained_bytes = frame.retained_bytes();
            cached.cursor = next_cursor;
            cached.snapshot_payload = payload;
            cached.snapshot_bytes = frame.payload_bytes.len();
            cached.snapshot_frame = frame.clone();
            cached.replay_events.push_back(ReplayableTopicEvent {
                frame: frame.clone(),
                bytes: retained_bytes,
                emitted_at: now,
            });
            cached.replay_bytes = cached.replay_bytes.saturating_add(retained_bytes);
            prune_replay_window(&mut cached.replay_events, &mut cached.replay_bytes);
            SubscriptionDispatchEvent { frame }
        };
        let _ = self.broadcaster.send(dispatch);
        Ok(())
    }
}
