use super::*;

pub(crate) fn build(
    rows: &[UpstreamAccountIdentityRow],
) -> (
    std::collections::HashMap<String, Vec<UpstreamAccountIdentityClusterMember>>,
    std::collections::HashMap<String, Vec<UpstreamAccountIdentityClusterMember>>,
) {
    let mut by_account_id =
        std::collections::HashMap::<String, Vec<UpstreamAccountIdentityClusterMember>>::new();
    let mut by_user_id =
        std::collections::HashMap::<String, Vec<UpstreamAccountIdentityClusterMember>>::new();
    for row in rows {
        let member = UpstreamAccountIdentityClusterMember {
            id: row.id,
            chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
            group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
            plan_type: normalize_plan_type(row.plan_type.as_deref()),
        };
        if let Some(account_id) = row.chatgpt_account_id.clone() {
            by_account_id
                .entry(account_id)
                .or_default()
                .push(member.clone());
        }
        if let Some(user_id) = row.chatgpt_user_id.clone() {
            by_user_id.entry(user_id).or_default().push(member);
        }
    }
    (by_account_id, by_user_id)
}

pub(crate) fn duplicate_info(
    rows: Vec<UpstreamAccountIdentityRow>,
    by_account_id: std::collections::HashMap<String, Vec<UpstreamAccountIdentityClusterMember>>,
    by_user_id: std::collections::HashMap<String, Vec<UpstreamAccountIdentityClusterMember>>,
) -> std::collections::HashMap<i64, DuplicateInfo> {
    let mut result = std::collections::HashMap::new();
    for row in rows {
        let current = UpstreamAccountIdentityClusterMember {
            id: row.id,
            chatgpt_user_id: normalize_optional_text(row.chatgpt_user_id.clone()),
            group_name: normalize_legacy_ungrouped_group_name(row.group_name.clone()),
            plan_type: normalize_plan_type(row.plan_type.as_deref()),
        };
        let mut peers = std::collections::BTreeSet::new();
        let mut reasons = Vec::new();
        if let Some(cluster) = row
            .chatgpt_account_id
            .as_ref()
            .and_then(|id| by_account_id.get(id))
            .filter(|v| v.len() > 1)
        {
            for member in cluster {
                if member.id != row.id
                    && should_flag_shared_account_id_duplicate_pair(&current, member)
                {
                    peers.insert(member.id);
                }
            }
            if !peers.is_empty() {
                reasons.push(DuplicateReason::SharedChatgptAccountId);
            }
        }
        if let Some(cluster) = row
            .chatgpt_user_id
            .as_ref()
            .and_then(|id| by_user_id.get(id))
            .filter(|v| v.len() > 1)
        {
            let matches = cluster.iter().any(|member| {
                member.id != row.id
                    && should_flag_duplicate_identity_pair(
                        current.plan_type.as_deref(),
                        member.plan_type.as_deref(),
                    )
            });
            for member in cluster {
                if member.id != row.id
                    && should_flag_duplicate_identity_pair(
                        current.plan_type.as_deref(),
                        member.plan_type.as_deref(),
                    )
                {
                    peers.insert(member.id);
                }
            }
            if matches {
                reasons.push(DuplicateReason::SharedChatgptUserId);
            }
        }
        if !peers.is_empty() {
            result.insert(
                row.id,
                DuplicateInfo {
                    peer_account_ids: peers.into_iter().collect(),
                    reasons,
                },
            );
        }
    }
    result
}
