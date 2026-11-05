mod schema;

pub(crate) use schema::*;

pub(crate) const ANALYTICS_MODULE_SCHEMA_SQL: &str = include_str!("schema/ddl.sql");
pub(crate) const ANALYTICS_MODULE_ROLE_GRANTS_SQL: &str = include_str!("schema/roles.sql");

/// A marker interface keeps module declarations uniform while later tasks add storage behavior.
pub(crate) trait AnalyticsModule {
    const ID: AnalyticsModuleId;
    const CONSUMER_ID: &'static str;
    const VERSION: i64;
    const DECLARATION: AnalyticsModuleDeclaration;
}

macro_rules! define_analytics_module {
    ($type:ident, $id:ident, $consumer:literal) => {
        pub(crate) struct $type;

        impl AnalyticsModule for $type {
            const ID: AnalyticsModuleId = AnalyticsModuleId::$id;
            const CONSUMER_ID: &'static str = $consumer;
            const VERSION: i64 = ANALYTICS_MODULE_VERSION;
            const DECLARATION: AnalyticsModuleDeclaration = AnalyticsModuleDeclaration {
                id: Self::ID,
                consumer_id: Self::CONSUMER_ID,
                version: Self::VERSION,
                reader_version: ANALYTICS_MODULE_READER_VERSION,
                requires_retention_pin: true,
            };
        }
    };
}

define_analytics_module!(InvocationAnalyticsModule, Invocation, "invocation");
define_analytics_module!(
    LongTermUsageAnalyticsModule,
    LongTermUsage,
    "long_term_usage"
);
define_analytics_module!(ParallelWorkAnalyticsModule, ParallelWork, "parallel_work");
define_analytics_module!(
    UpstreamAccountActivityAnalyticsModule,
    UpstreamAccountActivity,
    "upstream_account_activity"
);
define_analytics_module!(
    PromptCacheAnalyticsModule,
    PromptCacheStatistics,
    "prompt_cache_statistics"
);
define_analytics_module!(
    DashboardActivityAnalyticsModule,
    DashboardActivity,
    "dashboard_activity"
);
define_analytics_module!(NetworkAnalyticsModule, Network, "network");
define_analytics_module!(
    ForwardProxyAnalyticsModule,
    ForwardProxyStatistics,
    "forward_proxy_statistics"
);

pub(crate) const ANALYTICS_MODULE_DECLARATIONS: &[AnalyticsModuleDeclaration] = &[
    InvocationAnalyticsModule::DECLARATION,
    LongTermUsageAnalyticsModule::DECLARATION,
    ParallelWorkAnalyticsModule::DECLARATION,
    UpstreamAccountActivityAnalyticsModule::DECLARATION,
    PromptCacheAnalyticsModule::DECLARATION,
    DashboardActivityAnalyticsModule::DECLARATION,
    NetworkAnalyticsModule::DECLARATION,
    ForwardProxyAnalyticsModule::DECLARATION,
];

pub(crate) fn analytics_module_declaration(
    id: AnalyticsModuleId,
) -> Option<&'static AnalyticsModuleDeclaration> {
    ANALYTICS_MODULE_DECLARATIONS
        .iter()
        .find(|declaration| declaration.id == id)
}

pub(crate) trait AnalyticsModuleReader {
    fn declaration(&self) -> AnalyticsModuleDeclaration;

    fn accepts(&self, publication: &AnalyticsPublication) -> bool {
        let declaration = self.declaration();
        publication.module_id == declaration.id
            && publication.consumer_id == declaration.consumer_id
            && publication.state == AnalyticsPublicationState::Published
            && publication.reader_version >= declaration.reader_version
            && !publication.legacy_reader_fenced
    }
}

impl<T: AnalyticsModule> AnalyticsModuleReader for T {
    fn declaration(&self) -> AnalyticsModuleDeclaration {
        T::DECLARATION
    }
}

pub(crate) trait AnalyticsModuleWriter {
    fn declaration(&self) -> AnalyticsModuleDeclaration;

    fn owns(&self, owner: &AnalyticsOwner) -> bool {
        owner.status == AnalyticsOwnerStatus::Active
            && owner.owner_epoch > 0
            && owner.module_id == self.declaration().id
    }
}

impl<T: AnalyticsModule> AnalyticsModuleWriter for T {
    fn declaration(&self) -> AnalyticsModuleDeclaration {
        T::DECLARATION
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_registry_is_unique_and_complete() {
        assert_eq!(ANALYTICS_MODULE_DECLARATIONS.len(), 8);
        assert_eq!(
            AnalyticsModuleId::ALL.len(),
            ANALYTICS_MODULE_DECLARATIONS.len()
        );
        for (index, declaration) in ANALYTICS_MODULE_DECLARATIONS.iter().enumerate() {
            assert!(!declaration.consumer_id.is_empty());
            assert_eq!(declaration.version, ANALYTICS_MODULE_VERSION);
            assert!(AnalyticsModuleId::ALL.contains(&declaration.id));
            assert!(
                ANALYTICS_MODULE_DECLARATIONS[index + 1..]
                    .iter()
                    .all(|other| other.consumer_id != declaration.consumer_id)
            );
        }
    }

    #[test]
    fn reader_and_writer_fences_are_explicit() {
        let publication = AnalyticsPublication {
            module_id: AnalyticsModuleId::Invocation,
            publication_id: "pub-1".to_string(),
            consumer_id: "invocation".to_string(),
            build_id: "build-1".to_string(),
            owner_epoch: 4,
            published_through_change_id: 8,
            coverage_revision: 3,
            projection_version: 1,
            reader_version: ANALYTICS_MODULE_READER_VERSION,
            state: AnalyticsPublicationState::Published,
            legacy_reader_fenced: false,
        };
        let owner = AnalyticsOwner {
            module_id: AnalyticsModuleId::Invocation,
            owner_id: "owner-1".to_string(),
            source_instance_id: "source-1".to_string(),
            owner_epoch: 4,
            status: AnalyticsOwnerStatus::Active,
            build_id: Some("build-1".to_string()),
        };
        assert!(InvocationAnalyticsModule.accepts(&publication));
        assert!(InvocationAnalyticsModule.owns(&owner));
    }

    #[test]
    fn ddl_and_role_contracts_are_forward_only_and_least_privilege() {
        assert_eq!(ANALYTICS_MODULE_DDL_STEPS.len(), 1);
        assert_eq!(ANALYTICS_MODULE_DDL_STEPS[0].order, 1);
        assert!(ANALYTICS_MODULE_DDL_STEPS[0].requires_bootstrap);
        for relation in [
            "analytics_module_ddl_ledger",
            "analytics_module_consumers",
            "analytics_module_owners",
            "analytics_module_builds",
            "analytics_module_publications",
            "analytics_module_repairs",
        ] {
            assert!(ANALYTICS_MODULE_SCHEMA_SQL.contains(relation));
        }
        assert!(ANALYTICS_MODULE_SCHEMA_SQL.contains("CREATE TABLE IF NOT EXISTS"));
        assert!(!ANALYTICS_MODULE_SCHEMA_SQL.contains("DROP TABLE"));
        assert!(ANALYTICS_MODULE_ROLE_GRANTS_SQL.contains("REVOKE ALL ON TABLE"));
        assert!(ANALYTICS_MODULE_ROLE_GRANTS_SQL.contains("TO cvm_analytics_reader"));
        assert!(ANALYTICS_MODULE_ROLE_GRANTS_SQL.contains("TO cvm_analytics_maintenance"));
        assert!(!ANALYTICS_MODULE_ROLE_GRANTS_SQL.contains("GRANT CREATE"));
        assert!(!ANALYTICS_MODULE_ROLE_GRANTS_SQL.contains("GRANT ALL"));
    }
}
