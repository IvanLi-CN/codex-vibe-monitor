export const supportedLocales = ["zh", "en"] as const;

export type Locale = (typeof supportedLocales)[number];
export type TranslationValues = Record<string, string | number>;

const baseTranslations = {
  en: {
    "app.nav.dashboard": "Dashboard",
    "app.nav.stats": "Statistics",
    "app.nav.live": "Live",
    "app.nav.settings": "Settings",
    "app.nav.records": "Records",
    "app.nav.accountPool": "Account Pool",
    "app.brand": "Codex Vibe Monitor",
    "app.logoAlt": "Codex Vibe Monitor icon",
    "app.language.label": "Language",
    "app.language.option.en": "English",
    "app.language.option.zh": "中文",
    "app.language.switcherAria": "Change language",
    "app.theme.switcherAria": "Switch color theme",
    "app.theme.currentLight": "Light",
    "app.theme.currentDark": "Dark",
    "app.theme.switchToLight": "Switch to light mode",
    "app.theme.switchToDark": "Switch to dark mode",
    "app.proxySettings.button": "Proxy settings",
    "app.proxySettings.title": "Model list hijack",
    "app.proxySettings.description":
      "Control how /v1/models is exposed by the reverse proxy.",
    "app.proxySettings.loading": "Loading settings…",
    "app.proxySettings.loadError": "Failed to load settings: {{error}}",
    "app.proxySettings.saveError": "Failed to save settings: {{error}}",
    "app.proxySettings.hijackLabel": "Hijack /v1/models",
    "app.proxySettings.hijackHint":
      "Return preset models from this proxy instead of pure upstream passthrough.",
    "app.proxySettings.mergeLabel": "Merge upstream models in real time",
    "app.proxySettings.mergeHint":
      "When enabled, merge upstream models with presets and deduplicate by model id.",
    "app.proxySettings.mergeDisabledHint":
      "Enable hijack first to turn this option on.",
    "app.proxySettings.presetModels": "Preset model list",
    "app.proxySettings.enabledCount": "Enabled: {{count}} / {{total}}",
    "app.proxySettings.noneEnabledHint":
      "No preset model enabled. When hijack is on, no extra preset model is inserted.",
    "app.proxySettings.modelEnabledBadge": "Enabled",
    "app.proxySettings.modelDisabledBadge": "Disabled",
    "app.proxySettings.defaultOff": "Default: off",
    "app.proxySettings.saving": "Saving…",
    "app.proxySettings.close": "Close",
    "app.update.available": "A new version is available:",
    "app.update.current": "current",
    "app.update.refresh": "Refresh now",
    "app.update.later": "Later",
    "app.sse.banner.title": "Live connection lost",
    "app.sse.banner.description":
      "No server events have arrived for over two minutes.",
    "app.sse.banner.duration": "Offline for {{minutes}}m {{seconds}}s",
    "app.sse.banner.durationChip": "Offline {{minutes}}m {{seconds}}s",
    "app.sse.banner.retryIn": "Auto reconnect in {{seconds}}s",
    "app.sse.banner.retryingNow": "Retrying now…",
    "app.sse.banner.autoDisabled":
      "Auto reconnect paused after extended downtime. Click to try again.",
    "app.sse.banner.reconnectButton": "Reconnect now",
    "app.footer.githubAria": "Open GitHub repository",
    "app.footer.loadingVersion": "Loading version…",
    "app.footer.versionLabel": "{{scope}} {{version}}",
    "app.footer.frontendLabel": "Front-end",
    "app.footer.backendLabel": "Back-end",
    "app.footer.newVersionAvailable": "Page has a newer version",
    "app.footer.copyright": "© Codex Vibe Monitor",
    "accountPool.eyebrow": "Pool",
    "accountPool.title": "Account Pool",
    "accountPool.description":
      "Manage Codex upstream accounts, persistent login sessions, and normalized 5h / 7d quota snapshots.",
    "accountPool.nav.upstreamAccounts": "Upstream Accounts",
    "accountPool.nav.tags": "Tags",
    "accountPool.upstreamAccounts.title": "Upstream accounts",
    "accountPool.upstreamAccounts.description":
      "Add single OAuth, batch OAuth, and API key accounts, then keep their login state and quota snapshots healthy.",
    "accountPool.upstreamAccounts.listTitle": "Account roster",
    "accountPool.upstreamAccounts.listDescription":
      "Select an account to inspect identity, quota windows, and maintenance state.",
    "accountPool.upstreamAccounts.emptyTitle": "No upstream account yet",
    "accountPool.upstreamAccounts.emptyDescription":
      "Create an OAuth or API key account to start building the pool.",
    "accountPool.upstreamAccounts.detailEmptyTitle": "Choose an account",
    "accountPool.upstreamAccounts.detailEmptyDescription":
      "The detail panel will show login health, quota windows, and editable metadata.",
    "accountPool.upstreamAccounts.metrics.total": "Accounts",
    "accountPool.upstreamAccounts.metrics.oauth": "OAuth",
    "accountPool.upstreamAccounts.metrics.apiKey": "API keys",
    "accountPool.upstreamAccounts.metrics.attention": "Needs attention",
    "accountPool.upstreamAccounts.primaryWindowLabel": "5h window",
    "accountPool.upstreamAccounts.primaryWindowShortLabel": "5h",
    "accountPool.upstreamAccounts.secondaryWindowLabel": "7d window",
    "accountPool.upstreamAccounts.secondaryWindowShortLabel": "7d",
    "accountPool.upstreamAccounts.primaryWindowDescription":
      "Primary quota window, aligned with Codex 5-hour usage semantics.",
    "accountPool.upstreamAccounts.secondaryWindowDescription":
      "Secondary quota window, aligned with Codex 7-day usage semantics.",
    "accountPool.upstreamAccounts.limitLegendTitle": "Quota legend",
    "accountPool.upstreamAccounts.limitLegendDescription":
      "OAuth accounts show normalized upstream usage snapshots. API key accounts show local placeholder limits until routing metrics are wired in.",
    "accountPool.upstreamAccounts.routing.title":
      "Advanced routing & sync settings",
    "accountPool.upstreamAccounts.routing.description":
      "Edit the downstream pool API key and the tiered maintenance sync cadence for the account pool.",
    "accountPool.upstreamAccounts.routing.currentKey": "Current pool API key",
    "accountPool.upstreamAccounts.routing.edit": "Edit routing settings",
    "accountPool.upstreamAccounts.routing.close": "Close dialog",
    "accountPool.upstreamAccounts.routing.configured": "Configured",
    "accountPool.upstreamAccounts.routing.notConfigured": "Not configured",
    "accountPool.upstreamAccounts.routing.apiKeySectionTitle":
      "Pool route key",
    "accountPool.upstreamAccounts.routing.apiKeySectionDescription":
      "Optional. Leave blank to keep the current downstream pool API key unchanged.",
    "accountPool.upstreamAccounts.routing.apiKeyLabel":
      "Downstream pool API key",
    "accountPool.upstreamAccounts.routing.generate": "Generate key",
    "accountPool.upstreamAccounts.routing.apiKeyPlaceholder":
      "Paste a new pool API key to rotate the route target",
    "accountPool.upstreamAccounts.routing.maintenanceSectionTitle":
      "Tiered maintenance sync",
    "accountPool.upstreamAccounts.routing.maintenanceSectionDescription":
      "Healthy OAuth accounts with both windows available stay in the priority queue until the cap is reached; overflow accounts fall back to the secondary sync interval.",
    "accountPool.upstreamAccounts.routing.primarySyncIntervalLabel":
      "Priority sync interval",
    "accountPool.upstreamAccounts.routing.secondarySyncIntervalLabel":
      "Secondary sync interval",
    "accountPool.upstreamAccounts.routing.priorityCapLabel":
      "Priority available account cap",
    "accountPool.upstreamAccounts.routing.priorityCapValue":
      "Top {{count}} accounts",
    "accountPool.upstreamAccounts.routing.intervalHours": "{{count}}h",
    "accountPool.upstreamAccounts.routing.intervalMinutes": "{{count}}m",
    "accountPool.upstreamAccounts.routing.intervalSeconds": "{{count}}s",
    "accountPool.upstreamAccounts.routing.dialogTitle":
      "Advanced routing & sync settings",
    "accountPool.upstreamAccounts.routing.dialogDescription":
      "Edit the pool route key, request path timeouts, and the two-tier maintenance queue without touching environment variables.",
    "accountPool.upstreamAccounts.routing.save": "Save settings",
    "accountPool.upstreamAccounts.routing.validation.integerRequired":
      "Sync fields must be positive integers.",
    "accountPool.upstreamAccounts.routing.validation.primaryMin":
      "Priority sync interval must be at least 60 seconds.",
    "accountPool.upstreamAccounts.routing.validation.secondaryMin":
      "Secondary sync interval must be at least 60 seconds.",
    "accountPool.upstreamAccounts.routing.validation.secondaryAtLeastPrimary":
      "Secondary sync interval must be greater than or equal to the priority sync interval.",
    "accountPool.upstreamAccounts.routing.validation.priorityCapMin":
      "Priority available account cap must be at least 1.",
    "accountPool.upstreamAccounts.routing.timeout.sectionTitle":
      "Request path timeouts (seconds)",
    "accountPool.upstreamAccounts.routing.timeout.defaultFirstByte":
      "Default first byte",
    "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte":
      "/v1/responses first byte",
    "accountPool.upstreamAccounts.routing.timeout.upstreamHandshake":
      "Upstream handshake",
    "accountPool.upstreamAccounts.routing.timeout.compactHandshake":
      "Compact handshake",
    "accountPool.upstreamAccounts.routing.timeout.requestRead":
      "Request body read",
    "accountPool.upstreamAccounts.actions.refresh": "Refresh",
    "accountPool.upstreamAccounts.actions.addAccount": "Add account",
    "accountPool.upstreamAccounts.actions.addOauth": "Add OAuth account",
    "accountPool.upstreamAccounts.actions.addApiKey": "Add API key",
    "accountPool.upstreamAccounts.actions.addBatchOauth": "Batch OAuth",
    "accountPool.upstreamAccounts.actions.backToList": "Back to accounts",
    "accountPool.upstreamAccounts.actions.cancel": "Cancel",
    "accountPool.upstreamAccounts.actions.startOauth": "Start OAuth login",
    "accountPool.upstreamAccounts.actions.generateOauthUrl":
      "Generate OAuth URL",
    "accountPool.upstreamAccounts.actions.regenerateOauthUrl":
      "Regenerate OAuth URL",
    "accountPool.upstreamAccounts.actions.copyOauthUrl": "Copy OAuth URL",
    "accountPool.upstreamAccounts.actions.completeOauth":
      "Complete OAuth login",
    "accountPool.upstreamAccounts.actions.generateMailbox": "Generate",
    "accountPool.upstreamAccounts.actions.useMailboxAddress": "Use address",
    "accountPool.upstreamAccounts.actions.submitMailboxAddress":
      "Submit mailbox address",
    "accountPool.upstreamAccounts.actions.cancelMailboxEdit":
      "Cancel mailbox edit",
    "accountPool.upstreamAccounts.actions.copyMailbox": "Copy mailbox",
    "accountPool.upstreamAccounts.actions.copyMailboxHint": "Click to copy",
    "accountPool.upstreamAccounts.actions.copied": "Copied",
    "accountPool.upstreamAccounts.actions.manual": "Manual",
    "accountPool.upstreamAccounts.actions.manualCopyMailbox":
      "Auto copy failed. Please copy the mailbox below manually.",
    "accountPool.upstreamAccounts.actions.copyCode": "Copy code",
    "accountPool.upstreamAccounts.actions.copyInvite": "Copy invite",
    "accountPool.upstreamAccounts.actions.fetchMailboxStatus": "Fetch",
    "accountPool.upstreamAccounts.actions.createApiKey":
      "Create API key account",
    "accountPool.upstreamAccounts.actions.syncNow": "Sync now",
    "accountPool.upstreamAccounts.actions.relogin": "Re-authorize",
    "accountPool.upstreamAccounts.actions.delete": "Delete",
    "accountPool.upstreamAccounts.actions.confirmDelete": "Delete account",
    "accountPool.upstreamAccounts.actions.save": "Save changes",
    "accountPool.upstreamAccounts.actions.enable": "Enabled",
    "accountPool.upstreamAccounts.actions.openDetails": "Open details",
    "accountPool.upstreamAccounts.actions.dismissDuplicateWarning":
      "Dismiss warning",
    "accountPool.upstreamAccounts.actions.closeDetails": "Close details",
    "accountPool.upstreamAccounts.groupFilterLabel": "Account groups",
    "accountPool.upstreamAccounts.groupFilter.all": "All groups",
    "accountPool.upstreamAccounts.groupFilter.ungrouped": "Ungrouped",
    "accountPool.upstreamAccounts.groupFilterPlaceholder":
      "All groups or search group names",
    "accountPool.upstreamAccounts.groupFilterSearchPlaceholder":
      "Search groups...",
    "accountPool.upstreamAccounts.groupFilterEmpty": "No matching groups.",
    "accountPool.upstreamAccounts.groupFilterUseValue": 'Filter by "{{value}}"',
    "accountPool.upstreamAccounts.statusFilterLabel": "Account status",
    "accountPool.upstreamAccounts.statusFilter.all": "All statuses",
    "accountPool.upstreamAccounts.workStatusFilterLabel": "Work status",
    "accountPool.upstreamAccounts.workStatusFilter.all": "All work statuses",
    "accountPool.upstreamAccounts.enableStatusFilterLabel": "Enable status",
    "accountPool.upstreamAccounts.enableStatusFilter.all": "All enable statuses",
    "accountPool.upstreamAccounts.healthStatusFilterLabel": "Account health",
    "accountPool.upstreamAccounts.healthStatusFilter.all": "All account health statuses",
    "accountPool.upstreamAccounts.tagFilterLabel": "Account tags",
    "accountPool.upstreamAccounts.tagFilterPlaceholder": "All tags",
    "accountPool.upstreamAccounts.tagFilterSearchPlaceholder": "Search tags...",
    "accountPool.upstreamAccounts.tagFilterEmpty": "No matching tags.",
    "accountPool.upstreamAccounts.tagFilterClear": "Clear tag filters",
    "accountPool.upstreamAccounts.tagFilterAriaLabel":
      "Filter accounts by tags",
    "accountPool.upstreamAccounts.oauth.createTitle": "Codex OAuth login",
    "accountPool.upstreamAccounts.oauth.createDescription":
      "Generate a manual OAuth URL, copy it to another browser, and paste the localhost callback URL back here after sign-in.",
    "accountPool.upstreamAccounts.oauth.completed":
      "Authorization completed. The account list has been refreshed.",
    "accountPool.upstreamAccounts.oauth.failed":
      "Authorization did not finish. Check the upstream message and try again.",
    "accountPool.upstreamAccounts.oauth.popupFallback":
      "The popup was blocked, so the login page was opened in a new tab instead.",
    "accountPool.upstreamAccounts.oauth.popupClosed":
      "The popup was closed before the login session completed.",
    "accountPool.upstreamAccounts.oauth.openAgain": "Open login page again",
    "accountPool.upstreamAccounts.oauth.status.pending":
      "Waiting for OAuth callback",
    "accountPool.upstreamAccounts.oauth.status.completed":
      "OAuth callback completed",
    "accountPool.upstreamAccounts.oauth.status.failed": "OAuth login failed",
    "accountPool.upstreamAccounts.oauth.status.expired": "OAuth login expired",
    "accountPool.upstreamAccounts.createPage.title": "Add upstream account",
    "accountPool.upstreamAccounts.createPage.description":
      "Use one dedicated screen to create single OAuth, batch OAuth, or API key accounts without squeezing the roster view.",
    "accountPool.upstreamAccounts.createPage.relinkTitle":
      "Re-authorize upstream account",
    "accountPool.upstreamAccounts.createPage.relinkDescription":
      "Generate a fresh OAuth link for {{name}}, then paste the localhost callback URL here to keep the stored credentials valid.",
    "accountPool.upstreamAccounts.createPage.helpTitle": "Creation notes",
    "accountPool.upstreamAccounts.createPage.helpDescription":
      "Pick the account type first, then provide the metadata or local quota placeholders required for onboarding.",
    "accountPool.upstreamAccounts.createPage.tabsLabel": "Account type",
    "accountPool.upstreamAccounts.createPage.tabs.oauth": "OAuth login",
    "accountPool.upstreamAccounts.createPage.tabs.batchOauth": "Batch OAuth",
    "accountPool.upstreamAccounts.createPage.tabs.import": "Import JSON",
    "accountPool.upstreamAccounts.createPage.tabs.apiKey": "API key",
    "accountPool.upstreamAccounts.import.createTitle":
      "Import Codex OAuth JSON",
    "accountPool.upstreamAccounts.import.createDescription":
      "Select one or more exported Codex OAuth credential JSON files, validate them in batch, then import the usable accounts.",
    "accountPool.upstreamAccounts.import.fileInputLabel":
      "Credential JSON files",
    "accountPool.upstreamAccounts.import.selectedFilesTitle": "Selected files",
    "accountPool.upstreamAccounts.import.selectedFilesEmpty":
      "No JSON file selected yet.",
    "accountPool.upstreamAccounts.import.filesSelected":
      "{{count}} files selected",
    "accountPool.upstreamAccounts.import.clearSelection": "Clear selection",
    "accountPool.upstreamAccounts.import.defaultGroupPlaceholder":
      "Apply a default group to newly created imports",
    "accountPool.upstreamAccounts.import.defaultMetadataHint":
      "Default group notes and tags apply only when the import creates a brand-new account.",
    "accountPool.upstreamAccounts.import.validateAction": "Validate and review",
    "accountPool.upstreamAccounts.import.validation.title": "Import validation",
    "accountPool.upstreamAccounts.import.validation.description":
      "Checked {{checked}} of {{total}} unique credentials from {{files}} selected files.",
    "accountPool.upstreamAccounts.import.validation.checking":
      "Validating selected credential files…",
    "accountPool.upstreamAccounts.import.validation.empty":
      "No validation rows to show yet.",
    "accountPool.upstreamAccounts.import.validation.clearFilter":
      "Clear filter",
    "accountPool.upstreamAccounts.import.validation.resultsTitle":
      "Validation result list",
    "accountPool.upstreamAccounts.import.validation.resultsCount":
      "Showing {{shown}} of {{total}} rows.",
    "accountPool.upstreamAccounts.import.validation.metrics.files":
      "Selected files",
    "accountPool.upstreamAccounts.import.validation.metrics.unique":
      "Unique credentials",
    "accountPool.upstreamAccounts.import.validation.metrics.usable":
      "Usable now",
    "accountPool.upstreamAccounts.import.validation.metrics.review":
      "Needs review",
    "accountPool.upstreamAccounts.import.validation.columns.file":
      "File / identity",
    "accountPool.upstreamAccounts.import.validation.columns.result": "Result",
    "accountPool.upstreamAccounts.import.validation.columns.detail": "Detail",
    "accountPool.upstreamAccounts.import.validation.columns.actions": "Actions",
    "accountPool.upstreamAccounts.import.validation.matchedAccount":
      "Matches {{name}}",
    "accountPool.upstreamAccounts.import.validation.attempts":
      "Attempt {{count}}",
    "accountPool.upstreamAccounts.import.validation.noDetail":
      "No extra detail.",
    "accountPool.upstreamAccounts.import.validation.importedAccount":
      "Local account #{{id}}",
    "accountPool.upstreamAccounts.import.validation.retryOne": "Retry",
    "accountPool.upstreamAccounts.import.validation.retryFailed":
      "Retry failed",
    "accountPool.upstreamAccounts.import.validation.importValid":
      "Import usable files ({{count}})",
    "accountPool.upstreamAccounts.import.validation.footerHint":
      "{{valid}} files are ready to import. Duplicate-in-input rows: {{duplicates}}.",
    "accountPool.upstreamAccounts.import.validation.status.pending": "Checking",
    "accountPool.upstreamAccounts.import.validation.status.duplicate":
      "Duplicate",
    "accountPool.upstreamAccounts.import.validation.status.ok": "Ready",
    "accountPool.upstreamAccounts.import.validation.status.exhausted":
      "Importable (exhausted)",
    "accountPool.upstreamAccounts.import.validation.status.invalid": "Invalid",
    "accountPool.upstreamAccounts.import.validation.status.error": "Error",
    "accountPool.upstreamAccounts.import.validation.reportTitle":
      "Import report",
    "accountPool.upstreamAccounts.import.validation.reportReady": "Completed",
    "accountPool.upstreamAccounts.import.validation.report.created": "Created",
    "accountPool.upstreamAccounts.import.validation.report.updated":
      "Updated existing",
    "accountPool.upstreamAccounts.import.validation.report.failed": "Failed",
    "accountPool.upstreamAccounts.import.validation.report.selected":
      "Selected",
    "accountPool.upstreamAccounts.import.validation.reportResultsTitle":
      "Imported rows",
    "accountPool.upstreamAccounts.batchOauth.createTitle":
      "Batch Codex OAuth onboarding",
    "accountPool.upstreamAccounts.batchOauth.createDescription":
      "Fill the table, generate one OAuth URL per row, and complete callbacks independently without leaving this screen.",
    "accountPool.upstreamAccounts.batchOauth.tableTitle": "Batch OAuth table",
    "accountPool.upstreamAccounts.batchOauth.tableDescription":
      "Each logical account uses two table rows so every field stays single-line and scannable.",
    "accountPool.upstreamAccounts.batchOauth.tableAccountColumn": "Account",
    "accountPool.upstreamAccounts.batchOauth.tableFlowColumn": "OAuth flow",
    "accountPool.upstreamAccounts.batchOauth.statusHeader": "Status",
    "accountPool.upstreamAccounts.batchOauth.actionsHeader": "Row actions",
    "accountPool.upstreamAccounts.batchOauth.actions.addRow": "Add row",
    "accountPool.upstreamAccounts.batchOauth.defaultGroupLabel":
      "Default group",
    "accountPool.upstreamAccounts.batchOauth.defaultGroupPlaceholder":
      "Apply a default group to new rows",
    "accountPool.upstreamAccounts.batchOauth.actions.removeRow": "Remove row",
    "accountPool.upstreamAccounts.batchOauth.actions.expandNote": "Expand note",
    "accountPool.upstreamAccounts.batchOauth.actions.collapseNote":
      "Collapse note",
    "accountPool.upstreamAccounts.batchOauth.actions.toggleMother":
      "Toggle mother account",
    "accountPool.upstreamAccounts.batchOauth.actions.editMailbox":
      "Edit mailbox",
    "accountPool.upstreamAccounts.batchOauth.actions.submitMailbox":
      "Submit mailbox",
    "accountPool.upstreamAccounts.batchOauth.actions.cancelMailboxEdit":
      "Cancel mailbox edit",
    "accountPool.upstreamAccounts.batchOauth.validation.mailboxFormat":
      "Enter a valid email address before attaching it.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.generateTitle":
      "Generate OAuth URL",
    "accountPool.upstreamAccounts.batchOauth.tooltip.generateBody":
      "Generate a fresh login link for this row after the account metadata is ready.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateTitle":
      "Refresh OAuth URL",
    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateBody":
      "Use this when the old URL expired or the metadata changed. The previous login link should be considered invalid.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyTitle":
      "Copy OAuth URL",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyBody":
      "Copy the generated login URL, open it in the browser that will complete the login, and return here with the callback URL.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyCodeTitle":
      "Copy verification code",
    "accountPool.upstreamAccounts.batchOauth.tooltip.editMailboxTitle":
      "Edit mailbox",
    "accountPool.upstreamAccounts.batchOauth.tooltip.editMailboxBody":
      "Edit this row mailbox inside the popover, then submit the address to attach mailbox enhancements without leaving the table.",
    "accountPool.upstreamAccounts.batchOauth.codeMissing":
      "No verification code yet.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.invitedTitle":
      "Invite received",
    "accountPool.upstreamAccounts.batchOauth.tooltip.invitedBody":
      "This mailbox already received a workspace invite email.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedTitle":
      "No invite yet",
    "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedBody":
      "This mailbox has not received a workspace invite email yet.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.noteTitle":
      "Optional note",
    "accountPool.upstreamAccounts.batchOauth.tooltip.noteBody":
      "Store row-specific reminders here. This does not affect the OAuth flow and stays hidden unless you expand it.",
    "accountPool.upstreamAccounts.groupNotes.actions.edit": "Edit group note",
    "accountPool.upstreamAccounts.groupNotes.tooltip.body":
      "Edit the shared note for this group. Existing groups save immediately; brand-new groups stay local until an account actually lands in the group.",
    "accountPool.upstreamAccounts.groupNotes.dialogTitle": "Group note",
    "accountPool.upstreamAccounts.groupNotes.existingDescription":
      "This group already exists. Saving here updates the shared note for every account in the group immediately.",
    "accountPool.upstreamAccounts.groupNotes.draftDescription":
      "This group does not exist yet. Saving here stores a local draft and the note will be persisted when the first account is actually created in this group.",
    "accountPool.upstreamAccounts.groupNotes.notePlaceholder":
      "Write a shared note for this group",
    "accountPool.upstreamAccounts.groupNotes.badges.existing": "Saved group",
    "accountPool.upstreamAccounts.groupNotes.badges.draft": "Draft group",
    "accountPool.upstreamAccounts.batchOauth.tooltip.completeTitle":
      "Submit callback",
    "accountPool.upstreamAccounts.batchOauth.tooltip.completeBody":
      "After login succeeds in the browser, paste the callback URL into the field above, then submit this row to finish account creation.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.motherTitle":
      "Toggle mother account",
    "accountPool.upstreamAccounts.batchOauth.tooltip.motherBody":
      "Mark this row as the mother account for its group. Other drafts in the same group will immediately give up the crown.",
    "accountPool.upstreamAccounts.batchOauth.tooltip.removeTitle": "Remove row",
    "accountPool.upstreamAccounts.batchOauth.tooltip.removeBody":
      "Delete this draft row from the batch. Use this when you added an extra row by mistake.",
    "accountPool.upstreamAccounts.batchOauth.summary.total": "Rows",
    "accountPool.upstreamAccounts.batchOauth.summary.draft": "Draft",
    "accountPool.upstreamAccounts.batchOauth.summary.pending": "Pending",
    "accountPool.upstreamAccounts.batchOauth.summary.completed": "Completed",
    "accountPool.upstreamAccounts.batchOauth.summary.untitled": "Row {{index}}",
    "accountPool.upstreamAccounts.batchOauth.summary.quickHint":
      "Fill metadata first, then generate and complete OAuth for this row.",
    "accountPool.upstreamAccounts.batchOauth.status.draft": "Draft",
    "accountPool.upstreamAccounts.batchOauth.status.pending":
      "Waiting for callback",
    "accountPool.upstreamAccounts.batchOauth.status.completed": "Completed",
    "accountPool.upstreamAccounts.batchOauth.status.completedNeedsRefresh":
      "Needs refresh",
    "accountPool.upstreamAccounts.batchOauth.status.failed": "Failed",
    "accountPool.upstreamAccounts.batchOauth.status.expired": "Expired",
    "accountPool.upstreamAccounts.batchOauth.statusDetail.draft":
      "Fill the row metadata, generate an OAuth URL, then paste the callback URL back here.",
    "accountPool.upstreamAccounts.batchOauth.authUrlLabel": "Auth URL",
    "accountPool.upstreamAccounts.batchOauth.authUrlPlaceholder":
      "Generate an OAuth URL for this row first",
    "accountPool.upstreamAccounts.batchOauth.footerHint":
      "Completed rows stay visible on this page so you can finish the rest of the batch without losing context.",
    "accountPool.upstreamAccounts.batchOauth.regenerateRequired":
      "Metadata changed. Generate a fresh OAuth URL for this row before completing login.",
    "accountPool.upstreamAccounts.batchOauth.copyInlineFallback":
      "Copy failed. Select the Auth URL field and copy it manually.",
    "accountPool.upstreamAccounts.batchOauth.completed":
      "{{name}} is ready. Continue with the remaining rows when you are done here.",
    "accountPool.upstreamAccounts.batchOauth.completedNeedsRefresh":
      "OAuth completed on the server. Refresh the roster to load the final account details.",
    "accountPool.upstreamAccounts.apiKey.createTitle": "Codex API key account",
    "accountPool.upstreamAccounts.apiKey.createDescription":
      "Store a masked API key plus local placeholder limits for the 5-hour and 7-day windows.",
    "accountPool.upstreamAccounts.apiKey.localPlaceholder":
      "Local placeholder usage",
    "accountPool.upstreamAccounts.editTitle": "Editable profile",
    "accountPool.upstreamAccounts.editDescription":
      "Update display metadata, the per-account upstream base URL, local placeholder limits, or rotate the API key without deleting the account.",
    "accountPool.upstreamAccounts.healthTitle": "Login health",
    "accountPool.upstreamAccounts.healthDescription":
      "Keep the last successful sync, refresh, expiry, and error context visible so re-auth is never silent.",
    "accountPool.upstreamAccounts.stickyConversations.title":
      "Sticky key conversations",
    "accountPool.upstreamAccounts.stickyConversations.description":
      "Review the sticky keys currently attached to this upstream account, plus 24h request activity.",
    "accountPool.upstreamAccounts.stickyConversations.limitLabel":
      "Conversations",
    "accountPool.upstreamAccounts.stickyConversations.limitOption":
      "{{count}} conversations",
    "accountPool.upstreamAccounts.stickyConversations.empty":
      "No sticky key conversations are attached to this account yet.",
    "accountPool.upstreamAccounts.stickyConversations.chartAria":
      "24h token cumulative chart",
    "accountPool.upstreamAccounts.stickyConversations.table.stickyKey":
      "Sticky Key",
    "accountPool.upstreamAccounts.effectiveRule.title":
      "Effective routing rule",
    "accountPool.upstreamAccounts.effectiveRule.description":
      "This is the merged policy currently applied to the account after all selected tags are combined.",
    "accountPool.upstreamAccounts.effectiveRule.noTags":
      "No tag is attached, so the account keeps the default pool routing behavior.",
    "accountPool.upstreamAccounts.effectiveRule.guardEnabled":
      "Conversation guard on",
    "accountPool.upstreamAccounts.effectiveRule.guardDisabled":
      "Conversation guard off",
    "accountPool.upstreamAccounts.effectiveRule.allowCutOut": "Allow cut out",
    "accountPool.upstreamAccounts.effectiveRule.denyCutOut": "Block cut out",
    "accountPool.upstreamAccounts.effectiveRule.allowCutIn": "Allow cut in",
    "accountPool.upstreamAccounts.effectiveRule.denyCutIn": "Block cut in",
    "accountPool.upstreamAccounts.effectiveRule.sourceTags": "Rule source tags",
    "accountPool.upstreamAccounts.effectiveRule.guardRule":
      "Max {{count}} conversations within {{hours}} hour(s)",
    "accountPool.upstreamAccounts.effectiveRule.allGuardsApply":
      "All active guards must pass",
    "accountPool.upstreamAccounts.detailTitle": "Account details",
    "accountPool.upstreamAccounts.identityUnavailable":
      "Identity is not available yet.",
    "accountPool.upstreamAccounts.noHistory": "No quota history yet.",
    "accountPool.upstreamAccounts.noError": "No recent error.",
    "accountPool.upstreamAccounts.never": "Never",
    "accountPool.upstreamAccounts.unlimited": "Unlimited",
    "accountPool.upstreamAccounts.unavailable": "Unavailable",
    "accountPool.upstreamAccounts.writesDisabledTitle":
      "Write actions are disabled",
    "accountPool.upstreamAccounts.writesDisabledBody":
      "Set UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET before creating or updating upstream accounts, so refreshable credentials can stay encrypted on disk.",
    "accountPool.upstreamAccounts.deleteConfirm":
      "Delete {{name}} from the pool? This keeps no recovery copy.",
    "accountPool.upstreamAccounts.deleteConfirmTitle": "Delete {{name}}?",
    "accountPool.upstreamAccounts.kind.oauth": "OAuth",
    "accountPool.upstreamAccounts.kind.apiKey": "API key",
    "accountPool.upstreamAccounts.workStatus.working": "Working",
    "accountPool.upstreamAccounts.workStatus.idle": "Idle",
    "accountPool.upstreamAccounts.workStatus.rate_limited": "Rate limited",
    "accountPool.upstreamAccounts.enableStatus.enabled": "Enabled",
    "accountPool.upstreamAccounts.enableStatus.disabled": "Disabled",
    "accountPool.upstreamAccounts.healthStatus.normal": "Normal",
    "accountPool.upstreamAccounts.healthStatus.needs_reauth": "Needs re-auth",
    "accountPool.upstreamAccounts.healthStatus.upstream_unavailable":
      "Upstream unavailable",
    "accountPool.upstreamAccounts.healthStatus.upstream_rejected":
      "Upstream rejected",
    "accountPool.upstreamAccounts.healthStatus.error_other": "Other error",
    "accountPool.upstreamAccounts.syncState.idle": "Sync idle",
    "accountPool.upstreamAccounts.syncState.syncing": "Syncing",
    "accountPool.upstreamAccounts.status.active": "Active",
    "accountPool.upstreamAccounts.status.syncing": "Syncing",
    "accountPool.upstreamAccounts.status.needs_reauth": "Needs re-auth",
    "accountPool.upstreamAccounts.status.upstream_unavailable":
      "Upstream unavailable",
    "accountPool.upstreamAccounts.status.upstream_rejected":
      "Upstream rejected",
    "accountPool.upstreamAccounts.status.error_other": "Other error",
    "accountPool.upstreamAccounts.status.error": "Error",
    "accountPool.upstreamAccounts.status.disabled": "Disabled",
    "accountPool.upstreamAccounts.bulk.selectedCount":
      "{{count}} accounts selected across pages",
    "accountPool.upstreamAccounts.bulk.enable": "Enable",
    "accountPool.upstreamAccounts.bulk.disable": "Disable",
    "accountPool.upstreamAccounts.bulk.setGroup": "Set group",
    "accountPool.upstreamAccounts.bulk.addTags": "Add tags",
    "accountPool.upstreamAccounts.bulk.removeTags": "Remove tags",
    "accountPool.upstreamAccounts.bulk.sync": "Sync selected",
    "accountPool.upstreamAccounts.bulk.delete": "Delete selected",
    "accountPool.upstreamAccounts.bulk.clearSelection": "Clear selection",
    "accountPool.upstreamAccounts.bulk.selectPage": "Select current page",
    "accountPool.upstreamAccounts.bulk.selectRow": "Select {{name}}",
    "accountPool.upstreamAccounts.bulk.apply": "Apply",
    "accountPool.upstreamAccounts.bulk.actionLabel.enable": "Enable",
    "accountPool.upstreamAccounts.bulk.actionLabel.disable": "Disable",
    "accountPool.upstreamAccounts.bulk.actionLabel.delete": "Delete",
    "accountPool.upstreamAccounts.bulk.actionLabel.set_group": "Set group",
    "accountPool.upstreamAccounts.bulk.actionLabel.add_tags": "Add tags",
    "accountPool.upstreamAccounts.bulk.actionLabel.remove_tags":
      "Remove tags",
    "accountPool.upstreamAccounts.bulk.resultSummary":
      "{{action}} finished: {{succeeded}} succeeded, {{failed}} failed.",
    "accountPool.upstreamAccounts.bulk.syncProgressTitle":
      "Bulk sync progress",
    "accountPool.upstreamAccounts.bulk.syncProgressSummary":
      "{{completed}} / {{total}} done · {{succeeded}} succeeded · {{failed}} failed · {{skipped}} skipped",
    "accountPool.upstreamAccounts.bulk.cancelSync": "Cancel sync",
    "accountPool.upstreamAccounts.bulk.dismissSync": "Dismiss",
    "accountPool.upstreamAccounts.bulk.rowStatus.pending": "Pending",
    "accountPool.upstreamAccounts.bulk.rowStatus.succeeded": "Succeeded",
    "accountPool.upstreamAccounts.bulk.rowStatus.failed": "Failed",
    "accountPool.upstreamAccounts.bulk.rowStatus.skipped": "Skipped",
    "accountPool.upstreamAccounts.bulk.groupDialogTitle":
      "Set group for selected accounts",
    "accountPool.upstreamAccounts.bulk.groupDialogDescription":
      "Enter a group name to overwrite the selected accounts. Leave it empty to clear the group.",
    "accountPool.upstreamAccounts.bulk.groupField": "Target group",
    "accountPool.upstreamAccounts.bulk.groupPlaceholder":
      "Type a group name or leave empty to clear",
    "accountPool.upstreamAccounts.bulk.addTagsDialogTitle":
      "Add tags to selected accounts",
    "accountPool.upstreamAccounts.bulk.removeTagsDialogTitle":
      "Remove tags from selected accounts",
    "accountPool.upstreamAccounts.bulk.tagsDialogDescription":
      "Choose one or more existing tags for the selected accounts.",
    "accountPool.upstreamAccounts.bulk.tagsField": "Tags",
    "accountPool.upstreamAccounts.bulk.tagsPlaceholder": "Choose tags",
    "accountPool.upstreamAccounts.bulk.deleteDialogTitle":
      "Delete selected accounts",
    "accountPool.upstreamAccounts.bulk.deleteDialogDescription":
      "Delete {{count}} selected accounts? This cannot be undone.",
    "accountPool.upstreamAccounts.pagination.summary":
      "Page {{page}} / {{pageCount}} · {{total}} accounts",
    "accountPool.upstreamAccounts.pagination.pageSize": "Page size",
    "accountPool.upstreamAccounts.pagination.previous": "Previous",
    "accountPool.upstreamAccounts.pagination.next": "Next",
    "accountPool.upstreamAccounts.hints.dataPlaneUnavailableTitle":
      "The OAuth data plane is unavailable",
    "accountPool.upstreamAccounts.hints.dataPlaneUnavailableBody":
      "The main service could not reach the in-process OAuth Codex upstream. Check outbound connectivity to chatgpt.com and make sure this deployment is not still running the removed bridge-only build.",
    "accountPool.upstreamAccounts.hints.bridgeExchangeTitle":
      "This OAuth account still shows a legacy bridge error",
    "accountPool.upstreamAccounts.hints.bridgeExchangeBody":
      "The stored last_error came from the removed OAuth bridge path. A fresh sync or successful route should overwrite it; if the same text reappears, this deployment is still running an old build.",
    "accountPool.upstreamAccounts.hints.dataPlaneRejectedTitle":
      "The OAuth data plane rejected this request",
    "accountPool.upstreamAccounts.hints.dataPlaneRejectedBody":
      "The in-process OAuth Codex adapter reached the upstream data plane, but the request was rejected. Check the upstream message for missing scopes, permissions, or other account-specific failures before re-authorizing.",
    "accountPool.upstreamAccounts.hints.reauthTitle":
      "This OAuth account needs a fresh sign-in",
    "accountPool.upstreamAccounts.hints.reauthBody":
      "The upstream token or refresh grant is no longer valid. Re-authorize this account to issue a new token set.",
    "accountPool.upstreamAccounts.usage.primaryDescription":
      "Latest normalized primary window usage plus recent trend.",
    "accountPool.upstreamAccounts.usage.secondaryDescription":
      "Latest normalized secondary window usage plus recent trend.",
    "accountPool.upstreamAccounts.table.account": "Account",
    "accountPool.upstreamAccounts.table.lastSync": "Last success",
    "accountPool.upstreamAccounts.table.syncAndCall": "Sync / Call",
    "accountPool.upstreamAccounts.table.lastSuccessShort": "Sync",
    "accountPool.upstreamAccounts.table.lastCallShort": "Call",
    "accountPool.upstreamAccounts.table.windows": "Windows",
    "accountPool.upstreamAccounts.table.nextReset": "Reset",
    "accountPool.upstreamAccounts.table.nextResetCompact": "Reset",
    "accountPool.upstreamAccounts.table.off": "Off",
    "accountPool.upstreamAccounts.table.hiddenTagsA11y":
      "Show {{count}} hidden tags: {{names}}",
    "accountPool.upstreamAccounts.fields.displayName": "Display name",
    "accountPool.upstreamAccounts.fields.groupName": "Group",
    "accountPool.upstreamAccounts.fields.groupNamePlaceholder":
      "Select or type a group",
    "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder":
      "Search or create a group...",
    "accountPool.upstreamAccounts.fields.groupNameEmpty":
      "No existing group yet.",
    "accountPool.upstreamAccounts.fields.groupNameUseValue": 'Use "{{value}}"',
    "accountPool.tags.title": "Tag policies",
    "accountPool.tags.description":
      "Create reusable tags for upstream accounts and manage routing rules, account coverage, and group coverage in one place.",
    "accountPool.tags.actions.create": "Create tag",
    "accountPool.tags.listTitle": "Tag list",
    "accountPool.tags.listDescription":
      "Review every tag, its routing rule summary, and how many accounts or groups are affected.",
    "accountPool.tags.filters.search": "Search",
    "accountPool.tags.filters.searchPlaceholder": "Search tags by name",
    "accountPool.tags.filters.hasAccounts": "Account links",
    "accountPool.tags.filters.guardEnabled": "Conversation guard",
    "accountPool.tags.filters.cutOutBlocked": "Cut-out",
    "accountPool.tags.filters.cutInBlocked": "Cut-in",
    "accountPool.tags.filters.option.all": "All",
    "accountPool.tags.filters.option.linked": "Linked only",
    "accountPool.tags.filters.option.unlinked": "Unlinked only",
    "accountPool.tags.filters.option.guardOn": "Guard on",
    "accountPool.tags.filters.option.guardOff": "Guard off",
    "accountPool.tags.filters.option.allowed": "Allowed",
    "accountPool.tags.filters.option.blocked": "Blocked",
    "accountPool.tags.table.name": "Tag",
    "accountPool.tags.table.rule": "Routing rule",
    "accountPool.tags.table.accounts": "Accounts",
    "accountPool.tags.table.groups": "Groups",
    "accountPool.tags.table.updatedAt": "Updated",
    "accountPool.tags.rule.guard": "Max {{count}} conversations / {{hours}}h",
    "accountPool.tags.rule.guardOff": "Guard off",
    "accountPool.tags.rule.cutOutOn": "Cut-out allowed",
    "accountPool.tags.rule.cutOutOff": "Cut-out blocked",
    "accountPool.tags.rule.cutInOn": "Cut-in allowed",
    "accountPool.tags.rule.cutInOff": "Cut-in blocked",
    "accountPool.tags.field.label": "Tags",
    "accountPool.tags.field.add": "Add tag",
    "accountPool.tags.field.empty": "No tag selected yet.",
    "accountPool.tags.field.searchPlaceholder": "Search existing tags...",
    "accountPool.tags.field.searchEmpty": "No matching tags.",
    "accountPool.tags.field.createInline": 'Create "{{value}}"',
    "accountPool.tags.field.newTag": "new tag",
    "accountPool.tags.field.currentPage": "new",
    "accountPool.tags.field.remove": "Unlink tag",
    "accountPool.tags.field.deleteAndRemove": "Delete tag and unlink",
    "accountPool.tags.field.edit": "Edit rule",
    "accountPool.tags.dialog.createTitle": "Create tag",
    "accountPool.tags.dialog.editTitle": "Edit tag",
    "accountPool.tags.dialog.description":
      "Adjust the tag name and the routing rules that accounts under this tag must follow.",
    "accountPool.tags.dialog.name": "Tag name",
    "accountPool.tags.dialog.namePlaceholder":
      "For example: vip, night-shift, warm-standby",
    "accountPool.tags.dialog.guardEnabled":
      "Limit conversations within a rolling time window",
    "accountPool.tags.dialog.lookbackHours": "Lookback hours",
    "accountPool.tags.dialog.maxConversations": "Max conversations",
    "accountPool.tags.dialog.allowCutOut":
      "Allow moving conversations out of this account",
    "accountPool.tags.dialog.allowCutIn":
      "Allow moving conversations into this account",
    "accountPool.tags.dialog.cancel": "Cancel",
    "accountPool.tags.dialog.save": "Save tag",
    "accountPool.tags.dialog.createAction": "Create tag",
    "accountPool.tags.dialog.validation":
      "When the guard is enabled, both lookback hours and max conversations must be positive integers.",
    "accountPool.upstreamAccounts.oauth.generated":
      "OAuth URL is ready. It expires at {{expiresAt}}.",
    "accountPool.upstreamAccounts.oauth.copied":
      "OAuth URL copied. Complete sign-in elsewhere, then paste the callback URL here.",
    "accountPool.upstreamAccounts.oauth.copyFailed":
      "Copy failed. Use the manual copy panel instead.",
    "accountPool.upstreamAccounts.oauth.regenerateRequired":
      "Group note changed. Generate a fresh OAuth URL before completing login.",
    "accountPool.upstreamAccounts.oauth.manualFlowTitle":
      "Manual OAuth handoff",
    "accountPool.upstreamAccounts.oauth.manualFlowDescription":
      "Generate the OAuth URL here, copy it to the browser where you want to log in, then paste the final localhost callback URL back into this form.",
    "accountPool.upstreamAccounts.oauth.manualCopyTitle":
      "Manual copy required",
    "accountPool.upstreamAccounts.oauth.manualCopyDescription":
      "Automatic copy was blocked. The OAuth URL is selected below so you can copy it manually.",
    "accountPool.upstreamAccounts.oauth.callbackUrlLabel": "Callback URL",
    "accountPool.upstreamAccounts.oauth.callbackUrlDescription":
      "Paste the full localhost callback URL or its query string here, then finish OAuth login.",
    "accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder":
      "http://localhost:43210/oauth/callback?code=...&state=...",
    "accountPool.upstreamAccounts.oauth.mailboxHint":
      "Generate a temp mailbox or enter a MoeMail address here. Supported addresses keep code parsing and invite detection enabled.",
    "accountPool.upstreamAccounts.oauth.mailboxEmpty": "No mailbox yet",
    "accountPool.upstreamAccounts.oauth.mailboxInputPlaceholder":
      "Enter a supported mailbox address or generate a new one",
    "accountPool.upstreamAccounts.oauth.mailboxGenerated": "Generated mailbox",
    "accountPool.upstreamAccounts.oauth.mailboxAttached": "Attached mailbox",
    "accountPool.upstreamAccounts.oauth.mailboxExpired":
      "This temp mailbox has expired. Generate a fresh mailbox before waiting for new mail.",
    "accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable":
      "Mailbox status is unavailable right now. Generate a fresh mailbox if this keeps happening.",
    "accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed":
      "Mailbox refresh failed. We could not confirm the latest code or invite state.",
    "accountPool.upstreamAccounts.oauth.mailboxCheckingBadge": "Checking",
    "accountPool.upstreamAccounts.oauth.mailboxCheckFailedBadge":
      "Check failed",
    "accountPool.upstreamAccounts.oauth.refreshing":
      "Fetching the latest mailbox state...",
    "accountPool.upstreamAccounts.oauth.refreshingShort": "Fetching",
    "accountPool.upstreamAccounts.oauth.refreshIn":
      "Next refresh in {{seconds}}s",
    "accountPool.upstreamAccounts.oauth.refreshInShort": "{{seconds}}s",
    "accountPool.upstreamAccounts.oauth.refreshScheduledUnknown":
      "Waiting for the next refresh window",
    "accountPool.upstreamAccounts.oauth.receivedAt":
      "Received at {{timestamp}}",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedInvalidFormat":
      "This does not look like a valid email address, so mailbox enhancements stay disabled.",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedDomain":
      "This mailbox domain is not supported by the current MoeMail integration, so mailbox enhancements stay disabled.",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable":
      "This mailbox is not readable through the current MoeMail integration, so mailbox enhancements stay disabled.",
    "accountPool.upstreamAccounts.oauth.codeCardTitle": "Verification code",
    "accountPool.upstreamAccounts.oauth.codeCardEmpty":
      "No verification code detected yet.",
    "accountPool.upstreamAccounts.oauth.inviteCardTitle": "Invite summary",
    "accountPool.upstreamAccounts.oauth.inviteCardEmpty":
      "No invite detected yet.",
    "accountPool.upstreamAccounts.oauth.invitedState": "Invited",
    "accountPool.upstreamAccounts.oauth.notInvitedState": "Not invited",
    "accountPool.upstreamAccounts.fields.note": "Note",
    "accountPool.upstreamAccounts.fields.generatedMailbox": "Generated mailbox",
    "accountPool.upstreamAccounts.fields.generatedMailboxPlaceholder":
      "Generate a temp mailbox for this OAuth flow",
    "accountPool.upstreamAccounts.fields.mailboxAddress": "Mailbox address",
    "accountPool.upstreamAccounts.fields.email": "Email",
    "accountPool.upstreamAccounts.fields.accountId": "Account ID",
    "accountPool.upstreamAccounts.fields.userId": "User ID",
    "accountPool.upstreamAccounts.fields.primaryLimit": "5h local limit",
    "accountPool.upstreamAccounts.fields.secondaryLimit": "7d local limit",
    "accountPool.upstreamAccounts.fields.limitUnit": "Limit unit",
    "accountPool.upstreamAccounts.fields.apiKey": "API key",
    "accountPool.upstreamAccounts.fields.upstreamBaseUrl": "Upstream base URL",
    "accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder":
      "Leave blank to use OPENAI_UPSTREAM_BASE_URL",
    "accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid":
      "Use an absolute http(s) URL, for example https://proxy.example.com/gateway",
    "accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment":
      "Upstream base URL cannot include a query string or fragment.",
    "accountPool.upstreamAccounts.fields.rotateApiKey": "Rotate API key",
    "accountPool.upstreamAccounts.fields.rotateApiKeyPlaceholder":
      "Leave blank to keep the current key",
    "accountPool.upstreamAccounts.fields.lastSyncedAt": "Last sync",
    "accountPool.upstreamAccounts.fields.lastRefreshedAt": "Last refresh",
    "accountPool.upstreamAccounts.fields.tokenExpiresAt":
      "Access token expires",
    "accountPool.upstreamAccounts.fields.lastSuccessSync":
      "Last successful sync",
    "accountPool.upstreamAccounts.fields.credits": "Credits",
    "accountPool.upstreamAccounts.fields.compactSupport": "Compact support",
    "accountPool.upstreamAccounts.fields.compactObservedAt":
      "Compact observed at",
    "accountPool.upstreamAccounts.fields.compactReason": "Compact reason",
    "accountPool.upstreamAccounts.fields.lastError": "Last error",
    "accountPool.upstreamAccounts.table.latestActionShort": "Latest",
    "accountPool.upstreamAccounts.validation.displayNameDuplicate":
      "Display name must be unique.",
    "accountPool.upstreamAccounts.latestAction.title": "Latest account action",
    "accountPool.upstreamAccounts.latestAction.empty":
      "No account action has been recorded yet.",
    "accountPool.upstreamAccounts.latestAction.unknown": "Unknown",
    "accountPool.upstreamAccounts.latestAction.fields.action": "Action",
    "accountPool.upstreamAccounts.latestAction.fields.source": "Source",
    "accountPool.upstreamAccounts.latestAction.fields.reason": "Reason",
    "accountPool.upstreamAccounts.latestAction.fields.httpStatus":
      "HTTP status",
    "accountPool.upstreamAccounts.latestAction.fields.occurredAt":
      "Occurred at",
    "accountPool.upstreamAccounts.latestAction.fields.invokeId":
      "Invoke ID",
    "accountPool.upstreamAccounts.latestAction.fields.message": "Message",
    "accountPool.upstreamAccounts.compactSupport.supportedBadge":
      "Compact OK",
    "accountPool.upstreamAccounts.compactSupport.unsupportedBadge":
      "Compact unsupported",
    "accountPool.upstreamAccounts.compactSupport.status.supported":
      "Supported",
    "accountPool.upstreamAccounts.compactSupport.status.unsupported":
      "Unsupported",
    "accountPool.upstreamAccounts.compactSupport.status.unknown":
      "Unknown",
    "accountPool.upstreamAccounts.latestAction.actions.route_recovered":
      "Route recovered",
    "accountPool.upstreamAccounts.latestAction.actions.route_cooldown_started":
      "Route cooldown",
    "accountPool.upstreamAccounts.latestAction.actions.route_hard_unavailable":
      "Hard unavailable",
    "accountPool.upstreamAccounts.latestAction.actions.sync_succeeded":
      "Sync succeeded",
    "accountPool.upstreamAccounts.latestAction.actions.sync_recovery_blocked":
      "Recovery still blocked",
    "accountPool.upstreamAccounts.latestAction.actions.sync_failed":
      "Sync failed",
    "accountPool.upstreamAccounts.latestAction.actions.account_updated":
      "Account updated",
    "accountPool.upstreamAccounts.latestAction.sources.call": "Call",
    "accountPool.upstreamAccounts.latestAction.sources.sync_manual":
      "Manual sync",
    "accountPool.upstreamAccounts.latestAction.sources.sync_maintenance":
      "Maintenance sync",
    "accountPool.upstreamAccounts.latestAction.sources.sync_post_create":
      "Post-create sync",
    "accountPool.upstreamAccounts.latestAction.sources.oauth_import":
      "OAuth import",
    "accountPool.upstreamAccounts.latestAction.sources.account_update":
      "Account update",
    "accountPool.upstreamAccounts.latestAction.reasons.sync_ok":
      "Sync completed successfully",
    "accountPool.upstreamAccounts.latestAction.reasons.account_updated":
      "Account settings were updated",
    "accountPool.upstreamAccounts.latestAction.reasons.sync_error":
      "Sync failed",
    "accountPool.upstreamAccounts.latestAction.reasons.quota_still_exhausted":
      "Fresh usage snapshot still shows an exhausted limit window",
    "accountPool.upstreamAccounts.latestAction.reasons.recovery_unconfirmed_manual_required":
      "Manual recovery is required before the account can return to routing",
    "accountPool.upstreamAccounts.latestAction.reasons.transport_failure":
      "Transport failure",
    "accountPool.upstreamAccounts.latestAction.reasons.reauth_required":
      "Reauthentication required",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_401":
      "Upstream rejected credentials (401)",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_402":
      "Plan or billing rejected upstream access (402)",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_403":
      "Upstream rejected permissions (403)",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_429_rate_limit":
      "Upstream rate limited the account",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_429_quota_exhausted":
      "Upstream quota or weekly cap was exhausted",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_5xx":
      "Upstream service failure",
    "accountPool.upstreamAccounts.recentActions.title": "Recent account events",
    "accountPool.upstreamAccounts.recentActions.description":
      "Latest call and sync actions for this account.",
    "accountPool.upstreamAccounts.recentActions.empty":
      "No recent account events yet.",
    "accountPool.upstreamAccounts.duplicate.badge": "Duplicate",
    "accountPool.upstreamAccounts.duplicate.warningTitle":
      "{{name}} was saved, but the upstream identity looks duplicated.",
    "accountPool.upstreamAccounts.duplicate.warningBody":
      "Matched reasons: {{reasons}}. Related account ids: {{peers}}.",
    "accountPool.upstreamAccounts.duplicate.compactTitle":
      "Possible upstream duplicate",
    "accountPool.upstreamAccounts.duplicate.compactBody":
      "Matched: {{reasons}}. Related account ids: {{peers}}.",
    "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId":
      "shared ChatGPT account id",
    "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId":
      "shared ChatGPT user id",
    "accountPool.upstreamAccounts.mother.badge": "Mother",
    "accountPool.upstreamAccounts.mother.fieldLabel": "Mother account",
    "accountPool.upstreamAccounts.mother.notMother": "No",
    "accountPool.upstreamAccounts.mother.toggleLabel": "Use as mother account",
    "accountPool.upstreamAccounts.mother.toggleDescription":
      "Each group can keep only one mother account. Turning this on automatically moves the crown away from the previous owner.",
    "accountPool.upstreamAccounts.mother.notifications.title":
      "Mother account updated",
    "accountPool.upstreamAccounts.mother.notifications.undo": "Undo",
    "accountPool.upstreamAccounts.mother.notifications.dismiss": "Dismiss",
    "accountPool.upstreamAccounts.mother.notifications.replaced":
      "{{next}} is now the mother account for {{group}}. The crown moved from {{previous}}.",
    "accountPool.upstreamAccounts.mother.notifications.created":
      "{{next}} is now the mother account for {{group}}.",
    "accountPool.upstreamAccounts.mother.notifications.cleared":
      "{{previous}} is no longer the mother account for {{group}}.",
    "settings.title": "Settings",
    "settings.description":
      "Configure proxy behavior and pricing catalog for cost estimation.",
    "settings.loading": "Loading settings…",
    "settings.loadError": "Settings request failed: {{error}}",
    "settings.saving": "Saving…",
    "settings.autoSaved": "Auto save enabled",
    "settings.proxy.title": "Proxy configuration",
    "settings.proxy.description":
      "Control /v1/models hijack and upstream merge behavior.",
    "settings.proxy.hijackLabel": "Hijack /v1/models",
    "settings.proxy.hijackHint":
      "Return preset models from this proxy instead of pure upstream passthrough.",
    "settings.proxy.mergeLabel": "Merge upstream models in real time",
    "settings.proxy.mergeHint":
      "When enabled, merge upstream models with presets and deduplicate by model id.",
    "settings.proxy.mergeDisabledHint":
      "Enable hijack first to turn this option on.",
    "settings.proxy.presetModels": "Preset model list",
    "settings.proxy.enabledCount": "Enabled: {{count}} / {{total}}",
    "settings.proxy.noneEnabledHint":
      "No preset model enabled. Hijack mode will return no preset models.",
    "settings.proxy.fastModeRewriteLabel": "Fast mode request rewrite",
    "settings.proxy.fastModeRewriteHint":
      "Only affects POST /v1/responses and POST /v1/chat/completions. `requestedServiceTier` continues to show the final value sent upstream.",
    "settings.proxy.fastModeRewriteDisabled": "Disabled",
    "settings.proxy.fastModeRewriteFillMissing": "Fill missing as priority",
    "settings.proxy.fastModeRewriteForcePriority": "Force priority",
    "settings.proxy.upstream429RetriesLabel": "Upstream 429 auto-retry",
    "settings.proxy.upstream429RetriesHint":
      "Retry upstream 429 responses before failing over. Applies to all reverse-proxy upstream calls; 0 disables retries.",
    "settings.proxy.upstream429RetriesDisabled": "Disabled (0 retries)",
    "settings.proxy.upstream429RetriesOnce": "1 retry",
    "settings.proxy.upstream429RetriesMany": "{{count}} retries",
    "settings.forwardProxy.title": "Forward proxy routing",
    "settings.forwardProxy.description":
      "Configure upstream forwarding proxies, subscription refresh interval, and live health metrics.",
    "settings.forwardProxy.insertDirectLabel": "Insert direct connection",
    "settings.forwardProxy.insertDirectHint":
      "Keep a direct route as a special proxy entry for unified scheduling and fallback.",
    "settings.forwardProxy.proxyUrls": "Proxy URLs",
    "settings.forwardProxy.proxyUrlsPlaceholder":
      "Examples:\nhttp://127.0.0.1:7890\nsocks5://127.0.0.1:1080\nuser:pass@proxy.example.com:8443",
    "settings.forwardProxy.subscriptionUrls": "Subscription URLs",
    "settings.forwardProxy.subscriptionUrlsPlaceholder":
      "https://example.com/subscription.txt\nhttps://example.com/subscription.base64",
    "settings.forwardProxy.subscriptionInterval":
      "Subscription refresh interval",
    "settings.forwardProxy.interval.1m": "1 minute",
    "settings.forwardProxy.interval.5m": "5 minutes",
    "settings.forwardProxy.interval.15m": "15 minutes",
    "settings.forwardProxy.interval.1h": "1 hour",
    "settings.forwardProxy.interval.6h": "6 hours",
    "settings.forwardProxy.interval.1d": "1 day",
    "settings.forwardProxy.save": "Save routing config",
    "settings.forwardProxy.addProxyBatch": "Import nodes",
    "settings.forwardProxy.addSubscription": "Add subscription",
    "settings.forwardProxy.proxyCount": "Configured nodes: {{count}}",
    "settings.forwardProxy.subscriptionCount":
      "Configured subscriptions: {{count}}",
    "settings.forwardProxy.nodeItemFallback": "Node #{{index}}",
    "settings.forwardProxy.subscriptionItemFallback": "Subscription #{{index}}",
    "settings.forwardProxy.remove": "Remove",
    "settings.forwardProxy.listEmpty":
      "No entry yet. Add one after validation.",
    "settings.forwardProxy.subscriptionListEmpty": "No subscription yet.",
    "settings.forwardProxy.supportHint":
      "Supported schemes: http, https, socks, socks5, socks5h, vmess, vless, trojan, ss (via Xray)",
    "settings.forwardProxy.directLabel": "Direct",
    "settings.forwardProxy.modal.description":
      "Input a candidate and run validation first. Only validated entries can be added.",
    "settings.forwardProxy.modal.step1": "Step 1: Input nodes",
    "settings.forwardProxy.modal.step2": "Step 2: Validation results",
    "settings.forwardProxy.modal.step1Compact": "Input",
    "settings.forwardProxy.modal.step2Compact": "Review",
    "settings.forwardProxy.modal.proxyBatchTitle": "Import proxy nodes",
    "settings.forwardProxy.modal.subscriptionTitle": "Add subscription URL",
    "settings.forwardProxy.modal.proxyBatchInputLabel": "Proxy node lines",
    "settings.forwardProxy.modal.subscriptionInputLabel": "Subscription URL",
    "settings.forwardProxy.modal.proxyBatchPlaceholder":
      "One node per line:\nvmess://...\nvless://...\ntrojan://...\nss://...\nhttp://...\nsocks5://...",
    "settings.forwardProxy.modal.subscriptionPlaceholder":
      "https://example.com/subscription.base64",
    "settings.forwardProxy.modal.required": "Please enter a value first.",
    "settings.forwardProxy.modal.validating": "Validating candidate...",
    "settings.forwardProxy.modal.validate": "Validate",
    "settings.forwardProxy.modal.add": "Add",
    "settings.forwardProxy.modal.addNode": "Add node",
    "settings.forwardProxy.modal.retryNode": "Retry",
    "settings.forwardProxy.modal.submit": "Submit",
    "settings.forwardProxy.modal.submitWithCount": "Import {{count}} node(s)",
    "settings.forwardProxy.modal.backToStep1": "Back to input",
    "settings.forwardProxy.modal.cancel": "Cancel",
    "settings.forwardProxy.modal.validateSuccess": "Validation passed.",
    "settings.forwardProxy.modal.validateFailed": "Validation failed.",
    "settings.forwardProxy.modal.backendUnreachable":
      "Cannot reach backend service. Please ensure 127.0.0.1:8080 is running.",
    "settings.forwardProxy.modal.backendServerError":
      "Backend returned an internal error. Please check backend status and logs.",
    "settings.forwardProxy.modal.batchValidateSuccess":
      "Validation passed for {{count}} node(s).",
    "settings.forwardProxy.modal.batchValidateFailed":
      "Validation failed for {{failed}}/{{total}} node(s).",
    "settings.forwardProxy.modal.batchValidateSummary":
      "Available: {{available}}, unavailable: {{unavailable}}.",
    "settings.forwardProxy.modal.batchValidateProgress":
      "Validating: {{validating}}, available: {{available}}, unavailable: {{unavailable}}.",
    "settings.forwardProxy.modal.normalizedCount":
      "Normalized entries: {{count}}",
    "settings.forwardProxy.modal.normalizedValue": "Normalized: {{value}}",
    "settings.forwardProxy.modal.probeSummary":
      "Discovered: {{nodes}} node(s), probe latency: {{latency}}",
    "settings.forwardProxy.modal.unknownNode": "Unnamed node",
    "settings.forwardProxy.modal.unknownProtocol": "Unknown",
    "settings.forwardProxy.modal.rowValidating": "Validating...",
    "settings.forwardProxy.modal.resultIndex": "#",
    "settings.forwardProxy.modal.resultName": "Name",
    "settings.forwardProxy.modal.resultProtocol": "Protocol",
    "settings.forwardProxy.modal.resultNode": "Node",
    "settings.forwardProxy.modal.resultStatus": "Result",
    "settings.forwardProxy.modal.resultLatency": "Latency",
    "settings.forwardProxy.modal.resultAction": "Action",
    "settings.forwardProxy.modal.statusAvailable": "Available",
    "settings.forwardProxy.modal.statusUnavailable": "Unavailable",
    "settings.forwardProxy.modal.statusValidating": "Validating",
    "settings.forwardProxy.modal.statusTimeout": "Timeout",
    "settings.forwardProxy.modal.roundProgress": "R{{current}}/{{total}}",
    "settings.forwardProxy.modal.roundResultSuccess":
      "R{{round}} OK {{latency}}",
    "settings.forwardProxy.modal.roundResultTimeout": "R{{round}} Timeout",
    "settings.forwardProxy.modal.roundResultFailed": "R{{round}} Failed",
    "settings.forwardProxy.table.proxy": "Proxy",
    "settings.forwardProxy.table.oneMinute": "1m",
    "settings.forwardProxy.table.fifteenMinutes": "15m",
    "settings.forwardProxy.table.oneHour": "1h",
    "settings.forwardProxy.table.oneDay": "1d",
    "settings.forwardProxy.table.sevenDays": "7d",
    "settings.forwardProxy.table.weight": "Weight",
    "settings.forwardProxy.table.successRate": "Success: {{value}}",
    "settings.forwardProxy.table.avgLatency": "Avg latency: {{value}}",
    "settings.forwardProxy.table.empty": "No proxy entry available.",
    "settings.pricing.title": "Pricing configuration",
    "settings.pricing.description":
      "Edit model pricing used by new request cost estimation.",
    "settings.pricing.compactNote":
      "Compact requests reuse the matched model pricing for cost estimation.",
    "settings.pricing.add": "Add model",
    "settings.pricing.remove": "Remove",
    "settings.pricing.catalogVersion": "Catalog version",
    "settings.pricing.columns.model": "Model",
    "settings.pricing.columns.input": "Input / 1M",
    "settings.pricing.columns.output": "Output / 1M",
    "settings.pricing.columns.cacheInput": "Cached input / 1M",
    "settings.pricing.columns.reasoning": "Reasoning / 1M",
    "settings.pricing.columns.source": "Source",
    "settings.pricing.columns.actions": "Actions",
    "settings.pricing.errors.catalogVersionRequired":
      "Catalog version is required.",
    "settings.pricing.errors.modelRequired": "Model id is required.",
    "settings.pricing.errors.modelTooLong":
      "Model id cannot exceed 128 characters.",
    "settings.pricing.errors.modelDuplicated": "Model id must be unique.",
    "settings.pricing.errors.numberInvalid":
      "Pricing values must be valid numbers.",
    "settings.pricing.errors.numberNegative":
      "Pricing values must be non-negative.",
    "dashboard.section.summaryTitle": "Last 24 hours summary",
    "dashboard.activityOverview.title": "Activity Overview",
    "dashboard.activityOverview.range24h": "24 Hours",
    "dashboard.activityOverview.range7d": "7 Days",
    "dashboard.activityOverview.rangeToggleAria": "Switch activity range",
    "dashboard.section.liveRefreshing": "Live refresh",
    "dashboard.section.recentLiveTitle": "Latest {{count}} live records",
    "dashboard.today.title": "Today summary",
    "dashboard.today.subtitle": "Accumulated in natural day ({{timezone}})",
    "dashboard.today.dayBadge": "Today",
    "stats.range.lastHour": "Past hour",
    "stats.range.today": "Today",
    "stats.range.lastDay": "Past day",
    "stats.range.thisWeek": "This week",
    "stats.range.lastWeek": "Past 7 days",
    "stats.range.thisMonth": "This month",
    "stats.range.lastMonth": "Past month",
    "stats.bucket.eachMinute": "Each minute",
    "stats.bucket.each5Minutes": "Every 5 minutes",
    "stats.bucket.each15Minutes": "Every 15 minutes",
    "stats.bucket.each30Minutes": "Every 30 minutes",
    "stats.bucket.eachHour": "Each hour",
    "stats.bucket.each6Hours": "Every 6 hours",
    "stats.bucket.each12Hours": "Every 12 hours",
    "stats.bucket.each24Hours": "Every 24 hours",
    "stats.bucket.eachDay": "Each day",
    "stats.title": "Statistics",
    "stats.subtitle": "Select time range and aggregation granularity",
    "stats.settlementHour": "Settlement hour",
    "stats.trendTitle": "Trend",
    "stats.successFailureTitle": "Success vs failures",
    "stats.successFailure.legend.firstByteAvg": "First byte avg (ms)",
    "stats.successFailure.tooltip.successRate": "Success rate",
    "stats.successFailure.tooltip.firstByteAvg": "First byte avg",
    "stats.successFailure.tooltip.firstByteP95": "First byte P95",
    "stats.errors.title": "Error reasons",
    "stats.errors.scope.label": "Failure scope",
    "stats.errors.scope.service": "Service failures",
    "stats.errors.scope.client": "Client failures",
    "stats.errors.scope.abort": "Client aborts",
    "stats.errors.scope.all": "All failures",
    "stats.errors.summary.service": "Service",
    "stats.errors.summary.client": "Client",
    "stats.errors.summary.abort": "Abort",
    "stats.errors.summary.actionable": "Actionable",
    "stats.errors.actionableRate": "Actionable failure rate: {{rate}}",
    "quota.title": "Quota overview",
    "quota.subscription": "Subscription: {{name}}",
    "quota.status.active": "Active",
    "quota.labels.usageRate": "Usage rate",
    "quota.labels.used": "Used",
    "quota.labels.remaining": "Remaining quota",
    "quota.labels.nextReset": "Next reset",
    "quota.status.expired": "Expired",
    "quota.status.expireInDays": "Expires in {{count}} days",
    "quota.status.expireInHours": "Expires in {{count}} hours",
    "quota.status.expireInMinutes": "Expires in {{count}} minutes",
    "quota.status.expireAt": "Expires at {{time}}",
    "quota.status.expireUnknown": "Expires at —",
    "live.summary.current": "Current window",
    "live.summary.30m": "30 minutes",
    "live.summary.1h": "1 hour",
    "live.summary.1d": "1 day",
    "live.summary.title": "Live summary",
    "live.chart.title": "Live charts",
    "live.window.label": "Window size",
    "live.option.records": "{{count}} records",
    "live.latest.title": "Latest records",
    "live.conversations.title": "Prompt Cache Key conversations",
    "live.conversations.description":
      "Group requests by Prompt Cache Key. Filter by conversation count or recent activity window, while totals remain full-history metrics.",
    "live.conversations.selectionLabel": "Conversation filter",
    "live.conversations.option.count": "{{count}} conversations",
    "live.conversations.option.activityHours":
      "Active in last {{hours}} hour(s)",
    "live.conversations.empty": "No prompt cache key conversations yet.",
    "live.conversations.implicitFilter.inactiveOutside24h":
      "{{count}} newer conversation(s) were hidden because count mode only includes conversations active in the last 24 hours.",
    "live.conversations.implicitFilter.cappedTo50":
      "{{count}} conversation(s) matched the activity window but were hidden because activity-window mode shows at most 50 conversations.",
    "live.conversations.accountLabel.idFallback": "Account #{{id}}",
    "live.conversations.table.promptCacheKey": "Prompt Cache Key",
    "live.conversations.table.upstreamAccounts": "Upstream accounts",
    "live.conversations.table.summary": "Total",
    "live.conversations.table.requestCount": "Requests",
    "live.conversations.table.requestCountCompact": "requests",
    "live.conversations.table.totalTokens": "Total tokens",
    "live.conversations.table.totalTokensCompact": "Token",
    "live.conversations.table.totalCost": "Total cost",
    "live.conversations.table.time": "Time",
    "live.conversations.table.createdAt": "Created",
    "live.conversations.table.createdAtShort": "Created",
    "live.conversations.table.lastActivityAt": "Last activity",
    "live.conversations.table.lastActivityAtShort": "Active",
    "live.conversations.table.chart24h": "24h Token Cumulative",
    "live.conversations.table.chartWindow": "{{hours}}h Token Cumulative",
    "live.conversations.chartAria": "{{hours}}h token cumulative chart",
    "live.chart.tooltip.instructions":
      "Hover or tap for details. Focus the chart and use arrow keys to switch points.",
    "live.conversations.chart.tooltip.status": "Status",
    "live.conversations.chart.tooltip.requestTokens": "Request tokens",
    "live.conversations.chart.tooltip.cumulativeTokens": "Cumulative tokens",
    "live.proxy.title": "Proxy health",
    "live.proxy.description":
      "Per-node window stats (success rate / avg latency), last 24h request volume split by success/failure, and last 24h weight trend.",
    "live.proxy.table.proxy": "Proxy node",
    "live.proxy.table.oneMinute": "1m stats",
    "live.proxy.table.fifteenMinutes": "15m stats",
    "live.proxy.table.oneHour": "1h stats",
    "live.proxy.table.oneDay": "1d stats",
    "live.proxy.table.sevenDays": "7d stats",
    "live.proxy.table.trend24h": "Last 24h requests",
    "live.proxy.table.requestTrendAria": "Last 24h request volume chart",
    "live.proxy.table.weightTrend24h": "Last 24h weight trend",
    "live.proxy.table.weightTrendAria": "Last 24h weight trend chart",
    "live.proxy.table.requestTooltip.total": "Total requests",
    "live.proxy.table.weightTooltip.samples": "Samples",
    "live.proxy.table.weightTooltip.min": "Min",
    "live.proxy.table.weightTooltip.max": "Max",
    "live.proxy.table.weightTooltip.avg": "Avg",
    "live.proxy.table.weightTooltip.last": "Last",
    "live.proxy.table.successShort": "Success {{count}}",
    "live.proxy.table.failureShort": "Failure {{count}}",
    "live.proxy.table.currentWeight": "Weight {{value}}",
    "live.proxy.table.empty": "No proxy stats yet.",
    "records.title": "Request records",
    "records.subtitle":
      "Analyze requests from a stable search snapshot so rapid new data does not disturb paging.",
    "records.filters.title": "Filters",
    "records.filters.description":
      "Editing filters only changes the draft. Click Search to align to the latest snapshot.",
    "records.filters.rangePreset": "Time range",
    "records.filters.rangePreset.today": "Today",
    "records.filters.rangePreset.lastDay": "Past 24 hours",
    "records.filters.rangePreset.last7Days": "Past 7 days",
    "records.filters.rangePreset.last30Days": "Past 30 days",
    "records.filters.rangePreset.custom": "Custom range",
    "records.filters.from": "From",
    "records.filters.to": "To",
    "records.filters.status": "Status",
    "records.filters.status.all": "All statuses",
    "records.filters.status.success": "Success",
    "records.filters.status.failed": "Failed",
    "records.filters.status.running": "Running",
    "records.filters.status.pending": "Pending",
    "records.filters.any": "All",
    "records.filters.noMatches": "No matches",
    "records.filters.model": "Model",
    "records.filters.proxy": "Proxy",
    "records.filters.endpoint": "Endpoint",
    "records.filters.failureClass": "Failure class",
    "records.filters.failureClass.all": "All classes",
    "records.filters.failureClass.service": "Service failure",
    "records.filters.failureClass.client": "Client failure",
    "records.filters.failureClass.abort": "Client abort",
    "records.filters.upstreamScope": "Upstream",
    "records.filters.upstreamScope.all": "All upstreams",
    "records.filters.upstreamScope.external": "External",
    "records.filters.upstreamScope.internal": "Internal",
    "records.filters.failureKind": "Failure kind",
    "records.filters.promptCacheKey": "Prompt Cache Key",
    "records.filters.requesterIp": "Requester IP",
    "records.filters.keyword": "Keyword",
    "records.filters.minTotalTokens": "Min total tokens",
    "records.filters.maxTotalTokens": "Max total tokens",
    "records.filters.minTotalMs": "Min total ms",
    "records.filters.maxTotalMs": "Max total ms",
    "records.filters.search": "Search",
    "records.filters.searching": "Searching…",
    "records.filters.clearDraft": "Clear draft",
    "records.focus.label": "Records focus",
    "records.focus.token": "Token",
    "records.focus.network": "Network",
    "records.focus.exception": "Exception",
    "records.summary.title": "Summary",
    "records.summary.description":
      "Focus switches KPI cards and table columns only. It does not refresh the snapshot.",
    "records.summary.loadError": "Failed to load summary: {{error}}",
    "records.summary.notice.newData": "{{count}} new records",
    "records.summary.notice.refreshAction": "Load new data",
    "records.summary.notice.newDataAria":
      "{{count}} new records available. Activate to load them into the current snapshot.",
    "records.summary.notice.refreshAria":
      "Load {{count}} new records into the current snapshot.",
    "records.summary.notice.refreshingAria":
      "Loading {{count}} new records into the current snapshot.",
    "records.summary.token.requests": "Requests",
    "records.summary.token.totalTokens": "Total tokens",
    "records.summary.token.avgTokens": "Avg tokens / request",
    "records.summary.token.cacheInput": "Cached input tokens",
    "records.summary.token.totalCost": "Total cost",
    "records.summary.network.avgTtfb": "Avg TTFB",
    "records.summary.network.p95Ttfb": "P95 TTFB",
    "records.summary.network.avgTotal": "Avg total time",
    "records.summary.network.p95Total": "P95 total time",
    "records.summary.exception.failures": "Failures",
    "records.summary.exception.service": "Service failures",
    "records.summary.exception.client": "Client failures",
    "records.summary.exception.abort": "Client aborts",
    "records.summary.exception.actionable": "Actionable failures",
    "records.list.title": "Records",
    "records.list.description":
      "Sorting and paging stay inside the active snapshot until you search again.",
    "records.list.totalCount": "{{count}} records",
    "records.list.pageSize": "Page size",
    "records.list.sortBy": "Sort by",
    "records.list.sortOrder": "Order",
    "records.list.pageLabel": "Page {{page}} / {{totalPages}}",
    "records.list.prev": "Previous",
    "records.list.next": "Next",
    "records.list.sort.occurredAt": "Occurred time",
    "records.list.sort.totalTokens": "Total tokens",
    "records.list.sort.cost": "Cost",
    "records.list.sort.totalMs": "Total time",
    "records.list.sort.ttfb": "TTFB",
    "records.list.sort.status": "Status",
    "records.list.sort.asc": "Ascending",
    "records.list.sort.desc": "Descending",
    "records.table.loadError": "Failed to load records: {{error}}",
    "records.table.loadingAria": "Loading request records",
    "records.table.empty": "No records found in this snapshot.",
    "records.table.details": "Record details",
    "records.table.showDetails": "Show details",
    "records.table.hideDetails": "Hide details",
    "records.table.focusTitle": "Focus highlights",
    "records.table.token.inputCache": "Input / Cache",
    "records.table.token.outputReasoning": "Output / Reasoning",
    "records.table.token.totalTokens": "Total tokens",
    "records.table.token.cost": "Cost",
    "records.table.network.endpoint": "Endpoint",
    "records.table.network.requesterIp": "Requester IP",
    "records.table.network.ttfb": "TTFB",
    "records.table.network.firstResponseByteTotal": "System to first byte",
    "records.table.network.totalMs": "Total time",
    "records.table.exception.failureKind": "Failure kind",
    "records.table.exception.failureClass": "Failure class",
    "records.table.exception.actionable": "Actionable",
    "records.table.exception.actionableYes": "Actionable",
    "records.table.exception.actionableNo": "Not actionable",
    "records.table.exception.error": "Error",
    "metric.totalCount": "Calls",
    "metric.totalCost": "Cost",
    "metric.totalTokens": "Tokens",
    "legend.low": "Low",
    "legend.high": "High",
    "calendar.title": "Usage activity",
    "calendar.metricsToggleAria": "Switch metric",
    "calendar.valueSeparator": ": ",
    "calendar.timeZoneLabel": "Timezone",
    "calendar.weekday.sun": "Sun",
    "calendar.weekday.mon": "Mon",
    "calendar.weekday.tue": "Tue",
    "calendar.weekday.wed": "Wed",
    "calendar.weekday.thu": "Thu",
    "calendar.weekday.fri": "Fri",
    "calendar.weekday.sat": "Sat",
    "calendar.monthLabel": "{{year}}/{{month}}",
    "heatmap.title": "Last 7 days heatmap",
    "heatmap24h.title": "Last 24h heatmap (10-min)",
    "heatmap.metricsToggleAria": "Switch metric",
    "heatmap.noData": "No data yet",
    "table.loadError": "Failed to load records: {{error}}",
    "table.loadingRecordsAria": "Loading records",
    "table.noRecords": "No records yet.",
    "table.column.time": "Time",
    "table.column.model": "Model",
    "table.model.fastPriorityTitle": "Fast (priority processing)",
    "table.model.fastPriorityAria": "Fast mode enabled via priority processing",
    "table.model.fastRequestedOnlyTitle":
      "Fast requested, but priority processing was not applied",
    "table.model.fastRequestedOnlyAria":
      "Fast was requested, but priority processing was not applied",
    "table.column.account": "Account",
    "table.column.proxy": "Proxy",
    "table.column.status": "Status",
    "table.column.inputTokens": "Input",
    "table.column.outputTokens": "Output",
    "table.column.reasoningTokensShort": "Reasoning",
    "table.column.cacheInputTokens": "Cached input",
    "table.column.reasoningEffort": "Reasoning effort",
    "table.column.totalTokens": "Total tokens",
    "table.column.costUsd": "Cost (USD)",
    "table.column.latency": "Elapsed",
    "table.column.firstByteCompression": "First byte / HTTP compression",
    "table.column.firstResponseByteTotalCompression":
      "System to first byte / HTTP compression",
    "table.column.totalLatencyShort": "Elapsed",
    "table.column.firstByteLatencyShort": "TTFB",
    "table.column.firstResponseByteTotalShort": "First byte total",
    "table.column.httpCompressionShort": "HTTP",
    "table.latency.firstByteTotal": "First byte / Elapsed",
    "table.column.error": "Error",
    "table.account.reverseProxy": "Reverse proxy",
    "table.account.poolRoutingPending": "Pool routing...",
    "table.account.poolAccountUnavailable": "Pool account unavailable",
    "table.status.success": "Success",
    "table.status.failed": "Failed",
    "table.status.running": "Running",
    "table.status.pending": "Pending",
    "table.status.unknown": "Unknown",
    "table.detailsTitle": "Request details",
    "table.details.invokeId": "Invoke ID",
    "table.details.source": "Source",
    "table.details.account": "Account",
    "table.details.proxy": "Proxy",
    "table.details.endpoint": "Endpoint",
    "table.endpoint.responsesBadge": "Responses",
    "table.endpoint.chatBadge": "Chat",
    "table.endpoint.compactBadge": "Compact",
    "table.endpoint.compactHint": "Codex remote compaction request",
    "table.details.requesterIp": "Requester IP",
    "table.details.promptCacheKey": "Prompt Cache Key",
    "table.details.totalLatency": "Elapsed time",
    "table.details.firstByteLatency": "First-byte latency",
    "table.details.firstResponseByteTotal": "System to first byte",
    "table.details.httpCompression": "HTTP compression",
    "table.details.requestedServiceTier": "Requested service tier",
    "table.details.serviceTier": "Service tier",
    "table.details.reasoningEffort": "Reasoning effort",
    "table.details.reasoningTokens": "Reasoning tokens",
    "table.details.proxyWeightDelta": "Proxy weight delta (this call)",
    "table.details.proxyWeightDeltaA11yIncrease":
      "Proxy weight increased by {{value}}",
    "table.details.proxyWeightDeltaA11yDecrease":
      "Proxy weight decreased by {{value}}",
    "table.details.proxyWeightDeltaA11yUnchanged":
      "Proxy weight unchanged ({{value}})",
    "table.details.failureKind": "Failure kind",
    "table.details.streamTerminalEvent": "Stream terminal event",
    "table.details.upstreamErrorCode": "Upstream error code",
    "table.details.upstreamErrorMessage": "Upstream error message",
    "table.details.upstreamRequestId": "Upstream request ID",
    "table.details.poolAttemptCount": "Pool attempts",
    "table.details.poolDistinctAccountCount": "Distinct accounts",
    "table.details.poolAttemptTerminalReason": "Pool terminal reason",
    "table.details.timingsTitle": "Stage timings",
    "table.details.stage.requestRead": "Request read",
    "table.details.stage.requestParse": "Request parse",
    "table.details.stage.upstreamConnect": "Upstream connect",
    "table.details.stage.upstreamFirstByte": "Upstream first byte",
    "table.details.stage.upstreamStream": "Upstream stream",
    "table.details.stage.responseParse": "Response parse",
    "table.details.stage.persistence": "Persistence",
    "table.details.stage.total": "Total",
    "table.errorDetailsTitle": "Error details",
    "table.poolAttempts.title": "Pool attempts",
    "table.poolAttempts.loading": "Loading pool attempts",
    "table.poolAttempts.loadError": "Failed to load pool attempts: {{error}}",
    "table.poolAttempts.empty": "No pool attempt records found. The detail may already be cleaned.",
    "table.poolAttempts.notPool": "This request did not use pool routing.",
    "table.poolAttempts.retry": "Retry / account",
    "table.poolAttempts.httpStatus": "HTTP status",
    "table.poolAttempts.failureKind": "Failure kind",
    "table.poolAttempts.connectLatency": "Connect",
    "table.poolAttempts.firstByteLatency": "First byte",
    "table.poolAttempts.streamLatency": "Stream",
    "table.poolAttempts.startedAt": "Started at",
    "table.poolAttempts.finishedAt": "Finished at",
    "table.poolAttempts.upstreamRequestId": "Upstream request ID",
    "table.poolAttempts.status.success": "Success",
    "table.poolAttempts.status.httpFailure": "HTTP failure",
    "table.poolAttempts.status.transportFailure": "Transport failure",
    "table.poolAttempts.status.budgetExhaustedFinal": "Budget exhausted",
    "table.poolAttempts.status.unknown": "Unknown",
    "table.errorDetailsEmpty": "No error details available.",
    "table.accountDrawer.subtitle": "Upstream account",
    "table.accountDrawer.close": "Close account details",
    "table.accountDrawer.fallbackTitle": "Upstream account",
    "table.accountDrawer.errorTitle": "Failed to load upstream account details",
    "table.accountDrawer.emptyTitle": "Upstream account unavailable",
    "table.accountDrawer.emptyBody":
      "This account may have been removed, or the current request no longer links to an available account.",
    "table.accountDrawer.openAccountPool": "Open in account pool",
    "table.accountDrawer.healthTitle": "Health",
    "table.accountDrawer.healthDescription":
      "Recent sync, token, and error status for this upstream account.",
    "stats.cards.loadError": "Failed to load stats: {{error}}",
    "stats.cards.totalCalls": "Total calls",
    "stats.cards.success": "Success",
    "stats.cards.failures": "Failures",
    "stats.cards.totalCost": "Total cost",
    "stats.cards.totalTokens": "Total tokens",
    "chart.loading": "Loading",
    "chart.loadingDetailed": "Loading chart",
    "chart.noDataRange": "No data for selected range.",
    "chart.noDataPoints": "No data points yet.",
    "chart.totalTokens": "Total tokens",
    "chart.totalCost": "Cost (USD)",
    "chart.totalCount": "Calls",
    "unit.calls": "calls",
    "quota.status.expired.badge": "Expired",
  },
  zh: {
    "app.nav.dashboard": "总览",
    "app.nav.stats": "统计",
    "app.nav.live": "实况",
    "app.nav.settings": "设置",
    "app.nav.records": "记录",
    "app.nav.accountPool": "号池",
    "app.brand": "Codex Vibe Monitor",
    "app.logoAlt": "Codex Vibe Monitor 图标",
    "app.language.label": "语言",
    "app.language.option.en": "English",
    "app.language.option.zh": "中文",
    "app.language.switcherAria": "切换语言",
    "app.theme.switcherAria": "切换配色主题",
    "app.theme.currentLight": "浅色",
    "app.theme.currentDark": "深色",
    "app.theme.switchToLight": "切换到浅色模式",
    "app.theme.switchToDark": "切换到深色模式",
    "app.proxySettings.button": "代理设置",
    "app.proxySettings.title": "模型列表劫持",
    "app.proxySettings.description":
      "控制反向代理对 /v1/models 的对外返回行为。",
    "app.proxySettings.loading": "正在加载设置…",
    "app.proxySettings.loadError": "加载设置失败：{{error}}",
    "app.proxySettings.saveError": "保存设置失败：{{error}}",
    "app.proxySettings.hijackLabel": "劫持 /v1/models",
    "app.proxySettings.hijackHint":
      "开启后由当前代理返回预置模型列表，而不是纯透传上游。",
    "app.proxySettings.mergeLabel": "实时合并上游模型",
    "app.proxySettings.mergeHint":
      "开启后会实时请求上游模型并与预置列表按模型 ID 去重合并。",
    "app.proxySettings.mergeDisabledHint": "请先开启劫持，再启用该选项。",
    "app.proxySettings.presetModels": "预置模型列表",
    "app.proxySettings.enabledCount": "已启用：{{count}} / {{total}}",
    "app.proxySettings.noneEnabledHint":
      "当前没有启用任何预置模型；开启劫持后不会额外插入预置模型。",
    "app.proxySettings.modelEnabledBadge": "已启用",
    "app.proxySettings.modelDisabledBadge": "未启用",
    "app.proxySettings.defaultOff": "默认：关闭",
    "app.proxySettings.saving": "保存中…",
    "app.proxySettings.close": "关闭",
    "app.update.available": "有新版本可用：",
    "app.update.current": "当前",
    "app.update.refresh": "立即刷新",
    "app.update.later": "稍后",
    "app.sse.banner.title": "实时连接已中断",
    "app.sse.banner.description": "已超过 2 分钟未收到服务器事件。",
    "app.sse.banner.duration": "已掉线 {{minutes}} 分 {{seconds}} 秒",
    "app.sse.banner.durationChip": "已掉线 {{minutes}}分{{seconds}}秒",
    "app.sse.banner.retryIn": "{{seconds}} 秒后自动重连",
    "app.sse.banner.retryingNow": "正在尝试重连…",
    "app.sse.banner.autoDisabled": "长时间掉线后已暂停自动重连，请手动重试。",
    "app.sse.banner.reconnectButton": "立即重连",
    "app.footer.githubAria": "打开 GitHub 仓库",
    "app.footer.loadingVersion": "版本加载中…",
    "app.footer.versionLabel": "{{scope}} {{version}}",
    "app.footer.frontendLabel": "前端",
    "app.footer.backendLabel": "后端",
    "app.footer.newVersionAvailable": "页面有新版本",
    "app.footer.copyright": "© Codex Vibe Monitor",
    "accountPool.eyebrow": "号池",
    "accountPool.title": "号池",
    "accountPool.description":
      "集中管理 Codex 上游账号、持久登录状态，以及归一化后的 5 小时 / 7 天额度快照。",
    "accountPool.nav.upstreamAccounts": "上游账号",
    "accountPool.nav.tags": "标签",
    "accountPool.upstreamAccounts.title": "上游账号",
    "accountPool.upstreamAccounts.description":
      "新增单个 OAuth、批量 OAuth 与 API Key 账号，并持续维护登录状态和额度快照。",
    "accountPool.upstreamAccounts.listTitle": "账号列表",
    "accountPool.upstreamAccounts.listDescription":
      "选择一个账号，查看身份信息、额度窗口和维护状态。",
    "accountPool.upstreamAccounts.emptyTitle": "还没有上游账号",
    "accountPool.upstreamAccounts.emptyDescription":
      "先创建一个 OAuth 或 API Key 账号，把号池基础能力搭起来。",
    "accountPool.upstreamAccounts.detailEmptyTitle": "请选择一个账号",
    "accountPool.upstreamAccounts.detailEmptyDescription":
      "右侧会展示登录健康度、额度窗口和可编辑的账号信息。",
    "accountPool.upstreamAccounts.metrics.total": "账号总数",
    "accountPool.upstreamAccounts.metrics.oauth": "OAuth 账号",
    "accountPool.upstreamAccounts.metrics.apiKey": "API Key",
    "accountPool.upstreamAccounts.metrics.attention": "需关注",
    "accountPool.upstreamAccounts.primaryWindowLabel": "5 小时窗口",
    "accountPool.upstreamAccounts.primaryWindowShortLabel": "5h",
    "accountPool.upstreamAccounts.secondaryWindowLabel": "7 天窗口",
    "accountPool.upstreamAccounts.secondaryWindowShortLabel": "7d",
    "accountPool.upstreamAccounts.primaryWindowDescription":
      "主额度窗口，对齐 Codex 的 5 小时使用语义。",
    "accountPool.upstreamAccounts.secondaryWindowDescription":
      "副额度窗口，对齐 Codex 的 7 天使用语义。",
    "accountPool.upstreamAccounts.limitLegendTitle": "额度说明",
    "accountPool.upstreamAccounts.limitLegendDescription":
      "OAuth 账号展示上游归一化后的真实快照；API Key 账号在路由计量接入前，展示本地占位限额。",
    "accountPool.upstreamAccounts.routing.title": "高级路由与同步设置",
    "accountPool.upstreamAccounts.routing.description":
      "直接编辑号池下游 API Key，以及分层维护同步频率，不再依赖环境变量。",
    "accountPool.upstreamAccounts.routing.currentKey": "当前号池 API Key",
    "accountPool.upstreamAccounts.routing.edit": "编辑路由设置",
    "accountPool.upstreamAccounts.routing.close": "关闭弹窗",
    "accountPool.upstreamAccounts.routing.configured": "已配置",
    "accountPool.upstreamAccounts.routing.notConfigured": "未配置",
    "accountPool.upstreamAccounts.routing.apiKeySectionTitle": "号池路由密钥",
    "accountPool.upstreamAccounts.routing.apiKeySectionDescription":
      "可选。留空即可保留当前下游号池 API Key，不会强制你重新填写。",
    "accountPool.upstreamAccounts.routing.apiKeyLabel": "下游号池 API Key",
    "accountPool.upstreamAccounts.routing.generate": "生成密钥",
    "accountPool.upstreamAccounts.routing.apiKeyPlaceholder":
      "粘贴新的号池 API Key 以切换内部路由入口",
    "accountPool.upstreamAccounts.routing.maintenanceSectionTitle":
      "分层维护同步",
    "accountPool.upstreamAccounts.routing.maintenanceSectionDescription":
      "双窗口都有额度的健康 OAuth 账号会先进入优先队列，超过上限的账号自动落到次级同步频率。",
    "accountPool.upstreamAccounts.routing.primarySyncIntervalLabel":
      "优先队列同步间隔",
    "accountPool.upstreamAccounts.routing.secondarySyncIntervalLabel":
      "次级队列同步间隔",
    "accountPool.upstreamAccounts.routing.priorityCapLabel":
      "优先可用账号上限",
    "accountPool.upstreamAccounts.routing.priorityCapValue":
      "前 {{count}} 个账号",
    "accountPool.upstreamAccounts.routing.intervalHours": "{{count}} 小时",
    "accountPool.upstreamAccounts.routing.intervalMinutes": "{{count}} 分钟",
    "accountPool.upstreamAccounts.routing.intervalSeconds": "{{count}} 秒",
    "accountPool.upstreamAccounts.routing.dialogTitle": "高级路由与同步设置",
    "accountPool.upstreamAccounts.routing.dialogDescription":
      "在项目界面里直接编辑号池路由密钥、请求链路超时和双层 maintenance 队列参数。",
    "accountPool.upstreamAccounts.routing.save": "保存设置",
    "accountPool.upstreamAccounts.routing.validation.integerRequired":
      "同步字段必须填写为正整数。",
    "accountPool.upstreamAccounts.routing.validation.primaryMin":
      "优先队列同步间隔不能小于 60 秒。",
    "accountPool.upstreamAccounts.routing.validation.secondaryMin":
      "次级队列同步间隔不能小于 60 秒。",
    "accountPool.upstreamAccounts.routing.validation.secondaryAtLeastPrimary":
      "次级队列同步间隔必须大于等于优先队列同步间隔。",
    "accountPool.upstreamAccounts.routing.validation.priorityCapMin":
      "优先可用账号上限不能小于 1。",
    "accountPool.upstreamAccounts.routing.timeout.sectionTitle":
      "请求链路超时（秒）",
    "accountPool.upstreamAccounts.routing.timeout.defaultFirstByte":
      "默认首字节",
    "accountPool.upstreamAccounts.routing.timeout.responsesFirstByte":
      "/v1/responses 首字节",
    "accountPool.upstreamAccounts.routing.timeout.upstreamHandshake":
      "上游握手",
    "accountPool.upstreamAccounts.routing.timeout.compactHandshake":
      "Compact 握手",
    "accountPool.upstreamAccounts.routing.timeout.requestRead":
      "请求体读取",
    "accountPool.upstreamAccounts.actions.refresh": "刷新列表",
    "accountPool.upstreamAccounts.actions.addAccount": "新增账号",
    "accountPool.upstreamAccounts.actions.addOauth": "新增 OAuth 账号",
    "accountPool.upstreamAccounts.actions.addApiKey": "新增 API Key",
    "accountPool.upstreamAccounts.actions.addBatchOauth": "批量 OAuth",
    "accountPool.upstreamAccounts.actions.backToList": "返回账号列表",
    "accountPool.upstreamAccounts.actions.cancel": "取消",
    "accountPool.upstreamAccounts.actions.startOauth": "开始 OAuth 登录",
    "accountPool.upstreamAccounts.actions.generateOauthUrl": "生成 OAuth 地址",
    "accountPool.upstreamAccounts.actions.regenerateOauthUrl":
      "重新生成 OAuth 地址",
    "accountPool.upstreamAccounts.actions.copyOauthUrl": "复制 OAuth 地址",
    "accountPool.upstreamAccounts.actions.completeOauth": "完成 OAuth 登录",
    "accountPool.upstreamAccounts.actions.generateMailbox": "生成",
    "accountPool.upstreamAccounts.actions.useMailboxAddress": "使用地址",
    "accountPool.upstreamAccounts.actions.submitMailboxAddress": "提交邮箱地址",
    "accountPool.upstreamAccounts.actions.cancelMailboxEdit": "取消邮箱编辑",
    "accountPool.upstreamAccounts.actions.copyMailbox": "复制邮箱",
    "accountPool.upstreamAccounts.actions.copyMailboxHint": "点击复制",
    "accountPool.upstreamAccounts.actions.copied": "已复制",
    "accountPool.upstreamAccounts.actions.manual": "手动",
    "accountPool.upstreamAccounts.actions.manualCopyMailbox":
      "自动复制失败，请手动复制下面的邮箱地址。",
    "accountPool.upstreamAccounts.actions.copyCode": "复制验证码",
    "accountPool.upstreamAccounts.actions.copyInvite": "复制邀请",
    "accountPool.upstreamAccounts.actions.fetchMailboxStatus": "Fetch",
    "accountPool.upstreamAccounts.actions.createApiKey": "创建 API Key 账号",
    "accountPool.upstreamAccounts.actions.syncNow": "立即同步",
    "accountPool.upstreamAccounts.actions.relogin": "重新授权",
    "accountPool.upstreamAccounts.actions.delete": "删除",
    "accountPool.upstreamAccounts.actions.confirmDelete": "确认删除",
    "accountPool.upstreamAccounts.actions.save": "保存修改",
    "accountPool.upstreamAccounts.actions.enable": "启用",
    "accountPool.upstreamAccounts.actions.openDetails": "打开详情",
    "accountPool.upstreamAccounts.actions.dismissDuplicateWarning": "收起提示",
    "accountPool.upstreamAccounts.actions.closeDetails": "关闭详情",
    "accountPool.upstreamAccounts.groupFilterLabel": "账号分组",
    "accountPool.upstreamAccounts.groupFilter.all": "全部分组",
    "accountPool.upstreamAccounts.groupFilter.ungrouped": "未分组",
    "accountPool.upstreamAccounts.groupFilterPlaceholder":
      "全部分组或搜索分组名",
    "accountPool.upstreamAccounts.groupFilterSearchPlaceholder": "搜索分组...",
    "accountPool.upstreamAccounts.groupFilterEmpty": "没有匹配的分组。",
    "accountPool.upstreamAccounts.groupFilterUseValue": "按“{{value}}”筛选",
    "accountPool.upstreamAccounts.statusFilterLabel": "账号状态",
    "accountPool.upstreamAccounts.statusFilter.all": "全部状态",
    "accountPool.upstreamAccounts.workStatusFilterLabel": "工作状态",
    "accountPool.upstreamAccounts.workStatusFilter.all": "全部工作状态",
    "accountPool.upstreamAccounts.enableStatusFilterLabel": "启用状态",
    "accountPool.upstreamAccounts.enableStatusFilter.all": "全部启用状态",
    "accountPool.upstreamAccounts.healthStatusFilterLabel": "账号状态",
    "accountPool.upstreamAccounts.healthStatusFilter.all": "全部账号状态",
    "accountPool.upstreamAccounts.tagFilterLabel": "账号标签",
    "accountPool.upstreamAccounts.tagFilterPlaceholder": "全部标签",
    "accountPool.upstreamAccounts.tagFilterSearchPlaceholder": "搜索标签...",
    "accountPool.upstreamAccounts.tagFilterEmpty": "没有匹配的标签。",
    "accountPool.upstreamAccounts.tagFilterClear": "清空标签筛选",
    "accountPool.upstreamAccounts.tagFilterAriaLabel": "按标签筛选账号",
    "accountPool.upstreamAccounts.oauth.createTitle": "Codex OAuth 登录",
    "accountPool.upstreamAccounts.oauth.createDescription":
      "先生成手动 OAuth 授权地址，再复制到其他浏览器完成登录，最后把 localhost 回调地址粘贴回这里。",
    "accountPool.upstreamAccounts.oauth.completed":
      "授权完成，账号列表已刷新。",
    "accountPool.upstreamAccounts.oauth.failed":
      "授权流程没有完成，请检查上游返回信息后重试。",
    "accountPool.upstreamAccounts.oauth.popupFallback":
      "登录弹窗被浏览器拦截，已改为在新标签页打开授权页。",
    "accountPool.upstreamAccounts.oauth.popupClosed":
      "登录弹窗在完成授权前被关闭了。",
    "accountPool.upstreamAccounts.oauth.openAgain": "重新打开授权页",
    "accountPool.upstreamAccounts.oauth.status.pending": "等待 OAuth 回调",
    "accountPool.upstreamAccounts.oauth.status.completed": "OAuth 回调已完成",
    "accountPool.upstreamAccounts.oauth.status.failed": "OAuth 登录失败",
    "accountPool.upstreamAccounts.oauth.status.expired": "OAuth 登录已过期",
    "accountPool.upstreamAccounts.createPage.title": "新增账号",
    "accountPool.upstreamAccounts.createPage.description":
      "把单个 OAuth、批量 OAuth 和 API Key 账号创建拆到独立页面，避免挤占账号列表的浏览空间。",
    "accountPool.upstreamAccounts.createPage.relinkTitle": "重新授权账号",
    "accountPool.upstreamAccounts.createPage.relinkDescription":
      "为 {{name}} 重新生成 OAuth 地址，再把 localhost 回调链接贴回这里，保持已保存凭据可续期。",
    "accountPool.upstreamAccounts.createPage.helpTitle": "创建说明",
    "accountPool.upstreamAccounts.createPage.helpDescription":
      "先选账号类型，再填写必要的元数据或本地额度占位信息，完成后会回到账号列表。",
    "accountPool.upstreamAccounts.createPage.tabsLabel": "账号类型",
    "accountPool.upstreamAccounts.createPage.tabs.oauth": "OAuth 登录",
    "accountPool.upstreamAccounts.createPage.tabs.batchOauth": "批量 OAuth",
    "accountPool.upstreamAccounts.createPage.tabs.import": "导入 JSON",
    "accountPool.upstreamAccounts.createPage.tabs.apiKey": "API Key",
    "accountPool.upstreamAccounts.import.createTitle": "导入 Codex OAuth JSON",
    "accountPool.upstreamAccounts.import.createDescription":
      "选择一个或多个导出的 Codex OAuth 凭据 JSON 文件，先批量验活，再导入可用账号。",
    "accountPool.upstreamAccounts.import.fileInputLabel": "凭据 JSON 文件",
    "accountPool.upstreamAccounts.import.selectedFilesTitle": "已选文件",
    "accountPool.upstreamAccounts.import.selectedFilesEmpty":
      "还没有选择 JSON 文件。",
    "accountPool.upstreamAccounts.import.filesSelected":
      "已选择 {{count}} 个文件",
    "accountPool.upstreamAccounts.import.clearSelection": "清空选择",
    "accountPool.upstreamAccounts.import.defaultGroupPlaceholder":
      "为新建导入账号设置默认分组",
    "accountPool.upstreamAccounts.import.defaultMetadataHint":
      "默认分组备注和标签只会应用到新建账号，命中已有账号时不会覆盖现有元数据。",
    "accountPool.upstreamAccounts.import.validateAction": "验证并预览",
    "accountPool.upstreamAccounts.import.validation.title": "导入验证",
    "accountPool.upstreamAccounts.import.validation.description":
      "已检查 {{files}} 个文件中的 {{checked}} / {{total}} 个唯一凭据。",
    "accountPool.upstreamAccounts.import.validation.checking":
      "正在验证所选凭据文件…",
    "accountPool.upstreamAccounts.import.validation.empty":
      "暂时没有可显示的验证结果。",
    "accountPool.upstreamAccounts.import.validation.clearFilter": "清除筛选",
    "accountPool.upstreamAccounts.import.validation.resultsTitle":
      "验证结果列表",
    "accountPool.upstreamAccounts.import.validation.resultsCount":
      "当前显示 {{shown}} / {{total}} 行。",
    "accountPool.upstreamAccounts.import.validation.metrics.files": "已选文件",
    "accountPool.upstreamAccounts.import.validation.metrics.unique": "唯一凭据",
    "accountPool.upstreamAccounts.import.validation.metrics.usable":
      "当前可导入",
    "accountPool.upstreamAccounts.import.validation.metrics.review": "需要关注",
    "accountPool.upstreamAccounts.import.validation.columns.file":
      "文件 / 身份",
    "accountPool.upstreamAccounts.import.validation.columns.result": "结果",
    "accountPool.upstreamAccounts.import.validation.columns.detail": "详情",
    "accountPool.upstreamAccounts.import.validation.columns.actions": "操作",
    "accountPool.upstreamAccounts.import.validation.matchedAccount":
      "匹配到 {{name}}",
    "accountPool.upstreamAccounts.import.validation.attempts":
      "第 {{count}} 次",
    "accountPool.upstreamAccounts.import.validation.noDetail": "没有额外详情。",
    "accountPool.upstreamAccounts.import.validation.importedAccount":
      "本地账号 #{{id}}",
    "accountPool.upstreamAccounts.import.validation.retryOne": "重试此项",
    "accountPool.upstreamAccounts.import.validation.retryFailed": "重试失败项",
    "accountPool.upstreamAccounts.import.validation.importValid":
      "导入可用项（{{count}}）",
    "accountPool.upstreamAccounts.import.validation.footerHint":
      "当前有 {{valid}} 个文件可导入，输入内重复 {{duplicates}} 个。",
    "accountPool.upstreamAccounts.import.validation.status.pending": "校验中",
    "accountPool.upstreamAccounts.import.validation.status.duplicate":
      "输入重复",
    "accountPool.upstreamAccounts.import.validation.status.ok": "可导入",
    "accountPool.upstreamAccounts.import.validation.status.exhausted":
      "可导入（额度耗尽）",
    "accountPool.upstreamAccounts.import.validation.status.invalid": "无效",
    "accountPool.upstreamAccounts.import.validation.status.error": "错误",
    "accountPool.upstreamAccounts.import.validation.reportTitle": "导入报告",
    "accountPool.upstreamAccounts.import.validation.reportReady": "已完成",
    "accountPool.upstreamAccounts.import.validation.report.created": "新建",
    "accountPool.upstreamAccounts.import.validation.report.updated": "更新现有",
    "accountPool.upstreamAccounts.import.validation.report.failed": "失败",
    "accountPool.upstreamAccounts.import.validation.report.selected":
      "已选导入",
    "accountPool.upstreamAccounts.import.validation.reportResultsTitle":
      "导入明细",
    "accountPool.upstreamAccounts.batchOauth.createTitle":
      "批量 Codex OAuth 入池",
    "accountPool.upstreamAccounts.batchOauth.createDescription":
      "在表格中填写多行账号，逐行生成 OAuth 地址并分别完成 callback，无需离开当前页面。",
    "accountPool.upstreamAccounts.batchOauth.tableTitle": "批量 OAuth 表格",
    "accountPool.upstreamAccounts.batchOauth.tableDescription":
      "每个逻辑账号占两行视觉布局，确保所有字段都保持单行且易于扫描。",
    "accountPool.upstreamAccounts.batchOauth.tableAccountColumn": "账号信息",
    "accountPool.upstreamAccounts.batchOauth.tableFlowColumn": "OAuth 流程",
    "accountPool.upstreamAccounts.batchOauth.statusHeader": "状态",
    "accountPool.upstreamAccounts.batchOauth.actionsHeader": "行操作",
    "accountPool.upstreamAccounts.batchOauth.actions.addRow": "新增一行",
    "accountPool.upstreamAccounts.batchOauth.defaultGroupLabel": "默认分组",
    "accountPool.upstreamAccounts.batchOauth.defaultGroupPlaceholder":
      "给新增行设置默认分组",
    "accountPool.upstreamAccounts.batchOauth.actions.removeRow": "移除该行",
    "accountPool.upstreamAccounts.batchOauth.actions.expandNote": "展开备注",
    "accountPool.upstreamAccounts.batchOauth.actions.collapseNote": "收起备注",
    "accountPool.upstreamAccounts.batchOauth.actions.toggleMother":
      "切换母号标记",
    "accountPool.upstreamAccounts.batchOauth.actions.editMailbox": "编辑邮箱",
    "accountPool.upstreamAccounts.batchOauth.actions.submitMailbox": "提交邮箱",
    "accountPool.upstreamAccounts.batchOauth.actions.cancelMailboxEdit":
      "取消邮箱编辑",
    "accountPool.upstreamAccounts.batchOauth.validation.mailboxFormat":
      "请先填写格式正确的邮箱地址，再执行附着。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.generateTitle":
      "生成 OAuth 地址",
    "accountPool.upstreamAccounts.batchOauth.tooltip.generateBody":
      "先确认这一行的账号信息无误，再生成登录链接，后续就在这条链接上继续完成授权。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateTitle":
      "重新生成 OAuth 地址",
    "accountPool.upstreamAccounts.batchOauth.tooltip.regenerateBody":
      "当旧链接过期，或你改动了账号信息时，重新生成新的链接继续操作；旧链接应视为失效。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyTitle":
      "复制 OAuth 地址",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyBody":
      "把当前登录链接复制出去，在要完成登录的浏览器中打开；登录完成后，再把回调链接粘贴回这一行。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.copyCodeTitle":
      "复制验证码",
    "accountPool.upstreamAccounts.batchOauth.tooltip.editMailboxTitle":
      "编辑邮箱",
    "accountPool.upstreamAccounts.batchOauth.tooltip.editMailboxBody":
      "在悬浮气泡里直接编辑这行邮箱地址，提交后即可附着邮箱增强能力，不用离开表格。",
    "accountPool.upstreamAccounts.batchOauth.codeMissing": "还没有收到验证码。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.invitedTitle":
      "已收到邀请",
    "accountPool.upstreamAccounts.batchOauth.tooltip.invitedBody":
      "这个邮箱已经收到工作区邀请邮件。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedTitle":
      "暂未受邀",
    "accountPool.upstreamAccounts.batchOauth.tooltip.notInvitedBody":
      "这个邮箱暂时还没有收到工作区邀请邮件。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.noteTitle": "备注（可选）",
    "accountPool.upstreamAccounts.batchOauth.tooltip.noteBody":
      "只用于记录这行账号的附加说明，不影响 OAuth 流程；默认收起，避免占用表格空间。",
    "accountPool.upstreamAccounts.groupNotes.actions.edit": "编辑分组备注",
    "accountPool.upstreamAccounts.groupNotes.tooltip.body":
      "编辑这个分组的共享备注。已有分组会立即保存；全新分组会先保存在当前页面，等真正有账号落进该分组时再持久化。",
    "accountPool.upstreamAccounts.groupNotes.dialogTitle": "分组备注",
    "accountPool.upstreamAccounts.groupNotes.existingDescription":
      "这个分组已经存在，保存后会立即更新该分组下所有账号共用的备注。",
    "accountPool.upstreamAccounts.groupNotes.draftDescription":
      "这个分组还没有实际账号，当前保存只会写入本页草稿；等首个账号真正创建到该分组时才会持久化。",
    "accountPool.upstreamAccounts.groupNotes.notePlaceholder":
      "填写这个分组的共享备注",
    "accountPool.upstreamAccounts.groupNotes.badges.existing": "已存在分组",
    "accountPool.upstreamAccounts.groupNotes.badges.draft": "草稿分组",
    "accountPool.upstreamAccounts.batchOauth.tooltip.completeTitle": "提交回调",
    "accountPool.upstreamAccounts.batchOauth.tooltip.completeBody":
      "浏览器登录成功后，把上方回调链接粘贴完整，再点击这里完成这一行账号的入池。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.motherTitle": "切换母号",
    "accountPool.upstreamAccounts.batchOauth.tooltip.motherBody":
      "把这一行设为所在分组的母号；同组其他草稿行会立刻让出皇冠标记。",
    "accountPool.upstreamAccounts.batchOauth.tooltip.removeTitle": "移除该行",
    "accountPool.upstreamAccounts.batchOauth.tooltip.removeBody":
      "把当前草稿行从批量表格中删除；适合清理误加的空行或不再需要的账号行。",
    "accountPool.upstreamAccounts.batchOauth.summary.total": "总行数",
    "accountPool.upstreamAccounts.batchOauth.summary.draft": "草稿",
    "accountPool.upstreamAccounts.batchOauth.summary.pending": "待回调",
    "accountPool.upstreamAccounts.batchOauth.summary.completed": "已完成",
    "accountPool.upstreamAccounts.batchOauth.summary.untitled":
      "第 {{index}} 行",
    "accountPool.upstreamAccounts.batchOauth.summary.quickHint":
      "先填元数据，再为这一行生成并完成 OAuth。",
    "accountPool.upstreamAccounts.batchOauth.status.draft": "草稿",
    "accountPool.upstreamAccounts.batchOauth.status.pending": "等待回调",
    "accountPool.upstreamAccounts.batchOauth.status.completed": "已完成",
    "accountPool.upstreamAccounts.batchOauth.status.completedNeedsRefresh":
      "待刷新",
    "accountPool.upstreamAccounts.batchOauth.status.failed": "失败",
    "accountPool.upstreamAccounts.batchOauth.status.expired": "已过期",
    "accountPool.upstreamAccounts.batchOauth.statusDetail.draft":
      "先填写该行元数据，生成 OAuth 地址后再把 callback URL 粘贴回这里。",
    "accountPool.upstreamAccounts.batchOauth.authUrlLabel": "授权地址",
    "accountPool.upstreamAccounts.batchOauth.authUrlPlaceholder":
      "先为这一行生成 OAuth 地址",
    "accountPool.upstreamAccounts.batchOauth.footerHint":
      "已完成的行会保留在当前页面，方便继续处理剩余账号。",
    "accountPool.upstreamAccounts.batchOauth.regenerateRequired":
      "元数据已变更，请先为这一行重新生成 OAuth 地址再完成登录。",
    "accountPool.upstreamAccounts.batchOauth.copyInlineFallback":
      "复制失败，请直接选中授权地址字段手动复制。",
    "accountPool.upstreamAccounts.batchOauth.completed":
      "{{name}} 已就绪，你可以继续处理剩余行。",
    "accountPool.upstreamAccounts.batchOauth.completedNeedsRefresh":
      "服务端已完成 OAuth，请刷新账号列表以加载最终账号详情。",
    "accountPool.upstreamAccounts.apiKey.createTitle": "Codex API Key 账号",
    "accountPool.upstreamAccounts.apiKey.createDescription":
      "保存脱敏后的 API Key，并为 5 小时 / 7 天窗口录入本地占位限额。",
    "accountPool.upstreamAccounts.apiKey.localPlaceholder": "本地占位统计",
    "accountPool.upstreamAccounts.editTitle": "账号编辑",
    "accountPool.upstreamAccounts.editDescription":
      "更新显示名称、备注、账号级上游地址、本地限额，或者轮换 API Key，不必删除后重建。",
    "accountPool.upstreamAccounts.healthTitle": "登录健康度",
    "accountPool.upstreamAccounts.healthDescription":
      "持续保留最近成功同步、刷新、过期和错误上下文，避免账号静默掉线。",
    "accountPool.upstreamAccounts.stickyConversations.title": "Sticky Key 对话",
    "accountPool.upstreamAccounts.stickyConversations.description":
      "查看当前账号承接的 sticky key，以及最近 24 小时请求活跃情况。",
    "accountPool.upstreamAccounts.stickyConversations.limitLabel": "对话数量",
    "accountPool.upstreamAccounts.stickyConversations.limitOption":
      "{{count}} 个对话",
    "accountPool.upstreamAccounts.stickyConversations.empty":
      "这个账号暂时还没有关联的 sticky key 对话。",
    "accountPool.upstreamAccounts.stickyConversations.chartAria":
      "24 小时 Token 累计图",
    "accountPool.upstreamAccounts.stickyConversations.table.stickyKey":
      "Sticky Key",
    "accountPool.upstreamAccounts.effectiveRule.title": "最终生效规则",
    "accountPool.upstreamAccounts.effectiveRule.description":
      "这里展示的是当前账号在所有已关联 tag 合并后，真正参与路由判定的规则。",
    "accountPool.upstreamAccounts.effectiveRule.noTags":
      "当前没有关联 tag，所以这个账号仍使用号池默认路由行为。",
    "accountPool.upstreamAccounts.effectiveRule.guardEnabled": "会话上限已开启",
    "accountPool.upstreamAccounts.effectiveRule.guardDisabled":
      "会话上限未开启",
    "accountPool.upstreamAccounts.effectiveRule.allowCutOut": "允许切出",
    "accountPool.upstreamAccounts.effectiveRule.denyCutOut": "禁止切出",
    "accountPool.upstreamAccounts.effectiveRule.allowCutIn": "允许切入",
    "accountPool.upstreamAccounts.effectiveRule.denyCutIn": "禁止切入",
    "accountPool.upstreamAccounts.effectiveRule.sourceTags": "规则来源 tag",
    "accountPool.upstreamAccounts.effectiveRule.guardRule":
      "{{hours}} 小时内最多 {{count}} 个对话",
    "accountPool.upstreamAccounts.effectiveRule.allGuardsApply":
      "所有已开启的上限规则都会同时生效",
    "accountPool.upstreamAccounts.detailTitle": "账号详情",
    "accountPool.upstreamAccounts.identityUnavailable":
      "暂时还没有可展示的身份信息。",
    "accountPool.upstreamAccounts.noHistory": "还没有额度历史。",
    "accountPool.upstreamAccounts.noError": "最近没有错误。",
    "accountPool.upstreamAccounts.never": "从未",
    "accountPool.upstreamAccounts.unlimited": "无限制",
    "accountPool.upstreamAccounts.unavailable": "暂无",
    "accountPool.upstreamAccounts.writesDisabledTitle": "当前禁止写入",
    "accountPool.upstreamAccounts.writesDisabledBody":
      "请先配置 UPSTREAM_ACCOUNTS_ENCRYPTION_SECRET，再创建或修改上游账号，这样可续期凭据才能安全地加密落盘。",
    "accountPool.upstreamAccounts.deleteConfirm":
      "确认从号池中删除 {{name}} 吗？这一步不会保留恢复副本。",
    "accountPool.upstreamAccounts.deleteConfirmTitle": "确认删除 {{name}}？",
    "accountPool.upstreamAccounts.kind.oauth": "OAuth",
    "accountPool.upstreamAccounts.kind.apiKey": "API Key",
    "accountPool.upstreamAccounts.workStatus.working": "工作",
    "accountPool.upstreamAccounts.workStatus.idle": "空闲",
    "accountPool.upstreamAccounts.workStatus.rate_limited": "限流",
    "accountPool.upstreamAccounts.enableStatus.enabled": "启用",
    "accountPool.upstreamAccounts.enableStatus.disabled": "禁用",
    "accountPool.upstreamAccounts.healthStatus.normal": "正常",
    "accountPool.upstreamAccounts.healthStatus.needs_reauth": "需要重新授权",
    "accountPool.upstreamAccounts.healthStatus.upstream_unavailable": "上游不可达",
    "accountPool.upstreamAccounts.healthStatus.upstream_rejected": "上游拒绝",
    "accountPool.upstreamAccounts.healthStatus.error_other": "其它异常",
    "accountPool.upstreamAccounts.syncState.idle": "同步空闲",
    "accountPool.upstreamAccounts.syncState.syncing": "同步中",
    "accountPool.upstreamAccounts.status.active": "正常",
    "accountPool.upstreamAccounts.status.syncing": "同步中",
    "accountPool.upstreamAccounts.status.needs_reauth": "需要重新授权",
    "accountPool.upstreamAccounts.status.upstream_unavailable": "上游不可达",
    "accountPool.upstreamAccounts.status.upstream_rejected": "上游拒绝",
    "accountPool.upstreamAccounts.status.error_other": "其它异常",
    "accountPool.upstreamAccounts.status.error": "异常",
    "accountPool.upstreamAccounts.status.disabled": "已停用",
    "accountPool.upstreamAccounts.bulk.selectedCount":
      "已跨页选中 {{count}} 个账号",
    "accountPool.upstreamAccounts.bulk.enable": "批量启用",
    "accountPool.upstreamAccounts.bulk.disable": "批量停用",
    "accountPool.upstreamAccounts.bulk.setGroup": "设置分组",
    "accountPool.upstreamAccounts.bulk.addTags": "增加标签",
    "accountPool.upstreamAccounts.bulk.removeTags": "移除标签",
    "accountPool.upstreamAccounts.bulk.sync": "批量同步",
    "accountPool.upstreamAccounts.bulk.delete": "批量删除",
    "accountPool.upstreamAccounts.bulk.clearSelection": "清空选择",
    "accountPool.upstreamAccounts.bulk.selectPage": "选择当前页",
    "accountPool.upstreamAccounts.bulk.selectRow": "选择 {{name}}",
    "accountPool.upstreamAccounts.bulk.apply": "应用",
    "accountPool.upstreamAccounts.bulk.actionLabel.enable": "批量启用",
    "accountPool.upstreamAccounts.bulk.actionLabel.disable": "批量停用",
    "accountPool.upstreamAccounts.bulk.actionLabel.delete": "批量删除",
    "accountPool.upstreamAccounts.bulk.actionLabel.set_group": "设置分组",
    "accountPool.upstreamAccounts.bulk.actionLabel.add_tags": "增加标签",
    "accountPool.upstreamAccounts.bulk.actionLabel.remove_tags": "移除标签",
    "accountPool.upstreamAccounts.bulk.resultSummary":
      "{{action}}完成：成功 {{succeeded}} 个，失败 {{failed}} 个。",
    "accountPool.upstreamAccounts.bulk.syncProgressTitle": "批量同步进度",
    "accountPool.upstreamAccounts.bulk.syncProgressSummary":
      "已完成 {{completed}} / {{total}} · 成功 {{succeeded}} · 失败 {{failed}} · 跳过 {{skipped}}",
    "accountPool.upstreamAccounts.bulk.cancelSync": "取消同步",
    "accountPool.upstreamAccounts.bulk.dismissSync": "收起",
    "accountPool.upstreamAccounts.bulk.rowStatus.pending": "等待中",
    "accountPool.upstreamAccounts.bulk.rowStatus.succeeded": "成功",
    "accountPool.upstreamAccounts.bulk.rowStatus.failed": "失败",
    "accountPool.upstreamAccounts.bulk.rowStatus.skipped": "跳过",
    "accountPool.upstreamAccounts.bulk.groupDialogTitle": "批量设置分组",
    "accountPool.upstreamAccounts.bulk.groupDialogDescription":
      "输入分组名后会覆盖所选账号的分组；留空则清空分组。",
    "accountPool.upstreamAccounts.bulk.groupField": "目标分组",
    "accountPool.upstreamAccounts.bulk.groupPlaceholder":
      "输入分组名，留空则清空",
    "accountPool.upstreamAccounts.bulk.addTagsDialogTitle": "批量增加标签",
    "accountPool.upstreamAccounts.bulk.removeTagsDialogTitle":
      "批量移除标签",
    "accountPool.upstreamAccounts.bulk.tagsDialogDescription":
      "为所选账号选择一个或多个已有标签。",
    "accountPool.upstreamAccounts.bulk.tagsField": "标签",
    "accountPool.upstreamAccounts.bulk.tagsPlaceholder": "选择标签",
    "accountPool.upstreamAccounts.bulk.deleteDialogTitle": "批量删除账号",
    "accountPool.upstreamAccounts.bulk.deleteDialogDescription":
      "确认删除这 {{count}} 个已选账号吗？此操作不可恢复。",
    "accountPool.upstreamAccounts.pagination.summary":
      "第 {{page}} / {{pageCount}} 页，共 {{total}} 个账号",
    "accountPool.upstreamAccounts.pagination.pageSize": "每页",
    "accountPool.upstreamAccounts.pagination.previous": "上一页",
    "accountPool.upstreamAccounts.pagination.next": "下一页",
    "accountPool.upstreamAccounts.hints.dataPlaneUnavailableTitle":
      "OAuth 数据面当前不可用",
    "accountPool.upstreamAccounts.hints.dataPlaneUnavailableBody":
      "主服务暂时连不到内联 OAuth Codex 上游。先检查到 chatgpt.com 的出网连通性，并确认当前部署不是还停留在旧 bridge 版本。",
    "accountPool.upstreamAccounts.hints.bridgeExchangeTitle":
      "这个 OAuth 账号仍在展示历史 bridge 错误",
    "accountPool.upstreamAccounts.hints.bridgeExchangeBody":
      "当前展示的 last_error 来自已经移除的 OAuth bridge 链路。只要再完成一次成功同步或路由，它通常就会被覆盖；如果同样的文案再次出现，说明部署还在跑旧版本。",
    "accountPool.upstreamAccounts.hints.dataPlaneRejectedTitle":
      "OAuth 数据面拒绝了这次请求",
    "accountPool.upstreamAccounts.hints.dataPlaneRejectedBody":
      "内联 OAuth Codex adapter 已经连到数据面上游，但请求被拒绝了。先检查上游返回里的 scopes、权限或账号能力，再决定是否需要重新授权。",
    "accountPool.upstreamAccounts.hints.reauthTitle":
      "这个 OAuth 账号需要重新登录",
    "accountPool.upstreamAccounts.hints.reauthBody":
      "上游 token 或 refresh grant 已经失效。请重新授权这个账号，生成一套新的凭据。",
    "accountPool.upstreamAccounts.usage.primaryDescription":
      "最近一次归一化主窗口使用率，以及近期趋势。",
    "accountPool.upstreamAccounts.usage.secondaryDescription":
      "最近一次归一化副窗口使用率，以及近期趋势。",
    "accountPool.upstreamAccounts.table.account": "账号",
    "accountPool.upstreamAccounts.table.lastSync": "最近成功同步",
    "accountPool.upstreamAccounts.table.syncAndCall": "同步 / 调用",
    "accountPool.upstreamAccounts.table.lastSuccessShort": "同步",
    "accountPool.upstreamAccounts.table.lastCallShort": "调用",
    "accountPool.upstreamAccounts.table.windows": "窗口",
    "accountPool.upstreamAccounts.table.nextReset": "下次重置",
    "accountPool.upstreamAccounts.table.nextResetCompact": "重置",
    "accountPool.upstreamAccounts.table.off": "停用",
    "accountPool.upstreamAccounts.table.hiddenTagsA11y":
      "显示另外 {{count}} 个隐藏标签：{{names}}",
    "accountPool.upstreamAccounts.fields.displayName": "显示名称",
    "accountPool.upstreamAccounts.fields.groupName": "分组",
    "accountPool.upstreamAccounts.fields.groupNamePlaceholder":
      "选择已有分组，或直接输入新分组",
    "accountPool.upstreamAccounts.fields.groupNameSearchPlaceholder":
      "搜索或新建分组...",
    "accountPool.upstreamAccounts.fields.groupNameEmpty": "当前还没有分组。",
    "accountPool.upstreamAccounts.fields.groupNameUseValue": "使用“{{value}}”",
    "accountPool.tags.title": "Tag 路由策略",
    "accountPool.tags.description":
      "集中管理上游账号标签、路由规则摘要，以及它们覆盖的账号和账号分组范围。",
    "accountPool.tags.actions.create": "创建 tag",
    "accountPool.tags.listTitle": "Tag 列表",
    "accountPool.tags.listDescription":
      "快速查看每个 tag 的规则、关联账号数量和关联分组数量。",
    "accountPool.tags.filters.search": "搜索",
    "accountPool.tags.filters.searchPlaceholder": "按 tag 名称搜索",
    "accountPool.tags.filters.hasAccounts": "账号关联",
    "accountPool.tags.filters.guardEnabled": "会话上限",
    "accountPool.tags.filters.cutOutBlocked": "切出能力",
    "accountPool.tags.filters.cutInBlocked": "切入能力",
    "accountPool.tags.filters.option.all": "全部",
    "accountPool.tags.filters.option.linked": "仅已关联",
    "accountPool.tags.filters.option.unlinked": "仅未关联",
    "accountPool.tags.filters.option.guardOn": "仅已开启",
    "accountPool.tags.filters.option.guardOff": "仅未开启",
    "accountPool.tags.filters.option.allowed": "仅允许",
    "accountPool.tags.filters.option.blocked": "仅禁止",
    "accountPool.tags.table.name": "Tag",
    "accountPool.tags.table.rule": "路由规则",
    "accountPool.tags.table.accounts": "账号数",
    "accountPool.tags.table.groups": "分组数",
    "accountPool.tags.table.updatedAt": "更新时间",
    "accountPool.tags.rule.guard": "{{hours}} 小时 / {{count}} 个对话",
    "accountPool.tags.rule.guardOff": "未开启会话上限",
    "accountPool.tags.rule.cutOutOn": "允许切出",
    "accountPool.tags.rule.cutOutOff": "禁止切出",
    "accountPool.tags.rule.cutInOn": "允许切入",
    "accountPool.tags.rule.cutInOff": "禁止切入",
    "accountPool.tags.field.label": "Tags",
    "accountPool.tags.field.add": "添加 tag",
    "accountPool.tags.field.empty": "还没有选择任何 tag。",
    "accountPool.tags.field.searchPlaceholder": "搜索已有 tag...",
    "accountPool.tags.field.searchEmpty": "没有匹配的 tag。",
    "accountPool.tags.field.createInline": "创建“{{value}}”",
    "accountPool.tags.field.newTag": "新 tag",
    "accountPool.tags.field.currentPage": "本页新建",
    "accountPool.tags.field.remove": "取消关联",
    "accountPool.tags.field.deleteAndRemove": "删除并取消关联",
    "accountPool.tags.field.edit": "编辑规则",
    "accountPool.tags.dialog.createTitle": "创建 tag",
    "accountPool.tags.dialog.editTitle": "编辑 tag",
    "accountPool.tags.dialog.description":
      "设置 tag 名称，以及所有关联账号都要遵守的路由规则。",
    "accountPool.tags.dialog.name": "Tag 名称",
    "accountPool.tags.dialog.namePlaceholder":
      "例如：vip、night-shift、warm-standby",
    "accountPool.tags.dialog.guardEnabled": "限制滚动时间窗内的对话数量",
    "accountPool.tags.dialog.lookbackHours": "回看小时数",
    "accountPool.tags.dialog.maxConversations": "最大对话数",
    "accountPool.tags.dialog.allowCutOut": "允许把对话切出到其他账号",
    "accountPool.tags.dialog.allowCutIn": "允许把对话切入到当前账号",
    "accountPool.tags.dialog.cancel": "取消",
    "accountPool.tags.dialog.save": "保存 tag",
    "accountPool.tags.dialog.createAction": "创建 tag",
    "accountPool.tags.dialog.validation":
      "开启会话上限后，“回看小时数”和“最大对话数”都必须是正整数。",
    "accountPool.upstreamAccounts.oauth.generated":
      "OAuth 地址已生成，过期时间：{{expiresAt}}。",
    "accountPool.upstreamAccounts.oauth.copied":
      "OAuth 地址已复制。请在其他浏览器完成登录，再把回调链接粘贴回这里。",
    "accountPool.upstreamAccounts.oauth.copyFailed":
      "复制失败了，请使用手动复制面板。",
    "accountPool.upstreamAccounts.oauth.regenerateRequired":
      "分组备注已变更，请先重新生成 OAuth 地址再完成登录。",
    "accountPool.upstreamAccounts.oauth.manualFlowTitle": "手动 OAuth 交接",
    "accountPool.upstreamAccounts.oauth.manualFlowDescription":
      "先在这里生成 OAuth 地址，复制到你要登录的浏览器里完成授权，再把最终停在 localhost 的完整回调链接贴回表单。",
    "accountPool.upstreamAccounts.oauth.manualCopyTitle":
      "请手动复制 OAuth 地址",
    "accountPool.upstreamAccounts.oauth.manualCopyDescription":
      "当前浏览器拦截了自动复制，下面已经选中原始 OAuth 地址，直接手动复制即可。",
    "accountPool.upstreamAccounts.oauth.callbackUrlLabel": "回调链接",
    "accountPool.upstreamAccounts.oauth.callbackUrlDescription":
      "把完整的 localhost 回调链接，或者其中的查询串粘贴到这里，再完成 OAuth 登录。",
    "accountPool.upstreamAccounts.oauth.callbackUrlPlaceholder":
      "http://localhost:43210/oauth/callback?code=...&state=...",
    "accountPool.upstreamAccounts.oauth.mailboxHint":
      "这里既可以生成临时邮箱，也可以手动填写已接入的 MoeMail 邮箱；只有受支持地址才会继续启用验证码解析和邀请识别。",
    "accountPool.upstreamAccounts.oauth.mailboxEmpty": "还没有生成邮箱",
    "accountPool.upstreamAccounts.oauth.mailboxInputPlaceholder":
      "填写已支持的邮箱地址，或者直接生成一个新邮箱",
    "accountPool.upstreamAccounts.oauth.mailboxGenerated": "已生成邮箱",
    "accountPool.upstreamAccounts.oauth.mailboxAttached": "已附着邮箱",
    "accountPool.upstreamAccounts.oauth.mailboxExpired":
      "这个临时邮箱已经过期了。请重新生成一个新邮箱再等新邮件。",
    "accountPool.upstreamAccounts.oauth.mailboxStatusUnavailable":
      "暂时拿不到这个邮箱的状态。如果持续这样，建议重新生成一个新邮箱。",
    "accountPool.upstreamAccounts.oauth.mailboxStatusRefreshFailed":
      "邮箱状态刷新失败，暂时无法确认最新验证码或邀请状态。",
    "accountPool.upstreamAccounts.oauth.mailboxCheckingBadge": "查收中",
    "accountPool.upstreamAccounts.oauth.mailboxCheckFailedBadge": "查收失败",
    "accountPool.upstreamAccounts.oauth.refreshing": "正在拉取最新邮箱状态...",
    "accountPool.upstreamAccounts.oauth.refreshingShort": "拉取中",
    "accountPool.upstreamAccounts.oauth.refreshIn": "{{seconds}} 秒后自动刷新",
    "accountPool.upstreamAccounts.oauth.refreshInShort": "{{seconds}}秒",
    "accountPool.upstreamAccounts.oauth.refreshScheduledUnknown":
      "等待下一轮刷新",
    "accountPool.upstreamAccounts.oauth.receivedAt": "收到于 {{timestamp}}",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedInvalidFormat":
      "这个邮箱地址格式不正确，所以邮箱增强能力暂时不会启用。",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedDomain":
      "这个邮箱域名不在当前 MoeMail 集成支持范围内，所以邮箱增强能力暂时不会启用。",
    "accountPool.upstreamAccounts.oauth.mailboxUnsupportedNotReadable":
      "当前 MoeMail 集成暂时读不到这个邮箱，所以邮箱增强能力暂时不会启用。",
    "accountPool.upstreamAccounts.oauth.codeCardTitle": "验证码",
    "accountPool.upstreamAccounts.oauth.codeCardEmpty":
      "暂时还没有识别到验证码。",
    "accountPool.upstreamAccounts.oauth.inviteCardTitle": "邀请摘要",
    "accountPool.upstreamAccounts.oauth.inviteCardEmpty":
      "暂时还没有识别到邀请通知。",
    "accountPool.upstreamAccounts.oauth.invitedState": "已受邀",
    "accountPool.upstreamAccounts.oauth.notInvitedState": "未受邀",
    "accountPool.upstreamAccounts.fields.note": "备注",
    "accountPool.upstreamAccounts.fields.generatedMailbox": "生成邮箱",
    "accountPool.upstreamAccounts.fields.generatedMailboxPlaceholder":
      "为这次 OAuth 流程生成一个临时邮箱",
    "accountPool.upstreamAccounts.fields.mailboxAddress": "邮箱地址",
    "accountPool.upstreamAccounts.fields.email": "邮箱",
    "accountPool.upstreamAccounts.fields.accountId": "账号 ID",
    "accountPool.upstreamAccounts.fields.userId": "用户 ID",
    "accountPool.upstreamAccounts.fields.primaryLimit": "5 小时本地限额",
    "accountPool.upstreamAccounts.fields.secondaryLimit": "7 天本地限额",
    "accountPool.upstreamAccounts.fields.limitUnit": "限额单位",
    "accountPool.upstreamAccounts.fields.apiKey": "API Key",
    "accountPool.upstreamAccounts.fields.upstreamBaseUrl": "上游地址",
    "accountPool.upstreamAccounts.fields.upstreamBaseUrlPlaceholder":
      "留空则使用全局 OPENAI_UPSTREAM_BASE_URL",
    "accountPool.upstreamAccounts.validation.upstreamBaseUrlInvalid":
      "请填写 http(s) 的绝对 URL，例如 https://proxy.example.com/gateway",
    "accountPool.upstreamAccounts.validation.upstreamBaseUrlNoQueryOrFragment":
      "上游地址不能包含查询串或片段。",
    "accountPool.upstreamAccounts.fields.rotateApiKey": "轮换 API Key",
    "accountPool.upstreamAccounts.fields.rotateApiKeyPlaceholder":
      "留空则保持当前密钥不变",
    "accountPool.upstreamAccounts.fields.lastSyncedAt": "最近同步",
    "accountPool.upstreamAccounts.fields.lastRefreshedAt": "最近刷新",
    "accountPool.upstreamAccounts.fields.tokenExpiresAt": "访问令牌过期时间",
    "accountPool.upstreamAccounts.fields.lastSuccessSync": "最近成功同步",
    "accountPool.upstreamAccounts.fields.credits": "Credits",
    "accountPool.upstreamAccounts.fields.compactSupport": "Compact 支持",
    "accountPool.upstreamAccounts.fields.compactObservedAt": "Compact 最近观测",
    "accountPool.upstreamAccounts.fields.compactReason": "Compact 观测原因",
    "accountPool.upstreamAccounts.fields.lastError": "最近错误",
    "accountPool.upstreamAccounts.table.latestActionShort": "最近动作",
    "accountPool.upstreamAccounts.validation.displayNameDuplicate":
      "显示名称必须唯一。",
    "accountPool.upstreamAccounts.latestAction.title": "最近账号动作",
    "accountPool.upstreamAccounts.latestAction.empty":
      "暂时还没有记录到账号动作。",
    "accountPool.upstreamAccounts.latestAction.unknown": "未知",
    "accountPool.upstreamAccounts.compactSupport.supportedBadge":
      "Compact 可用",
    "accountPool.upstreamAccounts.compactSupport.unsupportedBadge":
      "Compact 不支持",
    "accountPool.upstreamAccounts.compactSupport.status.supported": "支持",
    "accountPool.upstreamAccounts.compactSupport.status.unsupported":
      "不支持",
    "accountPool.upstreamAccounts.compactSupport.status.unknown": "未知",
    "accountPool.upstreamAccounts.latestAction.fields.action": "动作",
    "accountPool.upstreamAccounts.latestAction.fields.source": "来源",
    "accountPool.upstreamAccounts.latestAction.fields.reason": "原因",
    "accountPool.upstreamAccounts.latestAction.fields.httpStatus": "HTTP 状态",
    "accountPool.upstreamAccounts.latestAction.fields.occurredAt": "发生时间",
    "accountPool.upstreamAccounts.latestAction.fields.invokeId": "调用 ID",
    "accountPool.upstreamAccounts.latestAction.fields.message": "消息",
    "accountPool.upstreamAccounts.latestAction.actions.route_recovered":
      "路由恢复成功",
    "accountPool.upstreamAccounts.latestAction.actions.route_cooldown_started":
      "进入冷却",
    "accountPool.upstreamAccounts.latestAction.actions.route_hard_unavailable":
      "标记为硬失效",
    "accountPool.upstreamAccounts.latestAction.actions.sync_succeeded":
      "同步成功",
    "accountPool.upstreamAccounts.latestAction.actions.sync_recovery_blocked":
      "恢复仍被阻止",
    "accountPool.upstreamAccounts.latestAction.actions.sync_failed": "同步失败",
    "accountPool.upstreamAccounts.latestAction.actions.account_updated":
      "账号已更新",
    "accountPool.upstreamAccounts.latestAction.sources.call": "调用",
    "accountPool.upstreamAccounts.latestAction.sources.sync_manual":
      "手动同步",
    "accountPool.upstreamAccounts.latestAction.sources.sync_maintenance":
      "维护同步",
    "accountPool.upstreamAccounts.latestAction.sources.sync_post_create":
      "创建后同步",
    "accountPool.upstreamAccounts.latestAction.sources.oauth_import":
      "OAuth 导入",
    "accountPool.upstreamAccounts.latestAction.sources.account_update":
      "账号修改",
    "accountPool.upstreamAccounts.latestAction.reasons.sync_ok": "同步完成",
    "accountPool.upstreamAccounts.latestAction.reasons.account_updated":
      "账号设置已更新",
    "accountPool.upstreamAccounts.latestAction.reasons.sync_error":
      "同步失败",
    "accountPool.upstreamAccounts.latestAction.reasons.quota_still_exhausted":
      "最新额度快照仍显示限制窗口已耗尽",
    "accountPool.upstreamAccounts.latestAction.reasons.recovery_unconfirmed_manual_required":
      "账号返回路由前仍需要人工恢复",
    "accountPool.upstreamAccounts.latestAction.reasons.transport_failure":
      "网络或传输失败",
    "accountPool.upstreamAccounts.latestAction.reasons.reauth_required":
      "需要重新登录",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_401":
      "上游拒绝凭据（401）",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_402":
      "上游因套餐或计费拒绝访问（402）",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_403":
      "上游拒绝权限（403）",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_429_rate_limit":
      "上游对该账号限流",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_429_quota_exhausted":
      "上游额度或周限已耗尽",
    "accountPool.upstreamAccounts.latestAction.reasons.upstream_http_5xx":
      "上游服务异常",
    "accountPool.upstreamAccounts.recentActions.title": "最近账号事件",
    "accountPool.upstreamAccounts.recentActions.description":
      "展示这条账号最近的调用与同步动作。",
    "accountPool.upstreamAccounts.recentActions.empty":
      "暂时还没有最近事件。",
    "accountPool.upstreamAccounts.duplicate.badge": "重复账号",
    "accountPool.upstreamAccounts.duplicate.warningTitle":
      "{{name}} 已保存，但检测到上游身份重复。",
    "accountPool.upstreamAccounts.duplicate.warningBody":
      "命中原因：{{reasons}}。关联账号 ID：{{peers}}。",
    "accountPool.upstreamAccounts.duplicate.compactTitle": "检测到上游身份重复",
    "accountPool.upstreamAccounts.duplicate.compactBody":
      "命中：{{reasons}}。关联账号 ID：{{peers}}。",
    "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptAccountId":
      "共享 ChatGPT 账号 ID",
    "accountPool.upstreamAccounts.duplicate.reasons.sharedChatgptUserId":
      "共享 ChatGPT 用户 ID",
    "accountPool.upstreamAccounts.mother.badge": "母号",
    "accountPool.upstreamAccounts.mother.fieldLabel": "母号状态",
    "accountPool.upstreamAccounts.mother.notMother": "否",
    "accountPool.upstreamAccounts.mother.toggleLabel": "设为母号",
    "accountPool.upstreamAccounts.mother.toggleDescription":
      "每个分组只能保留一个母号。开启后会自动把同组旧母号的皇冠切走。",
    "accountPool.upstreamAccounts.mother.notifications.title": "母号已更新",
    "accountPool.upstreamAccounts.mother.notifications.undo": "撤销",
    "accountPool.upstreamAccounts.mother.notifications.dismiss": "关闭",
    "accountPool.upstreamAccounts.mother.notifications.replaced":
      "{{group}} 的母号已切换为 {{next}}，原母号 {{previous}} 已自动退位。",
    "accountPool.upstreamAccounts.mother.notifications.created":
      "{{next}} 已成为 {{group}} 的母号。",
    "accountPool.upstreamAccounts.mother.notifications.cleared":
      "{{previous}} 已不再是 {{group}} 的母号。",
    "settings.title": "设置",
    "settings.description": "集中配置代理行为与价格目录，用于成本估算。",
    "settings.loading": "正在加载设置…",
    "settings.loadError": "设置请求失败：{{error}}",
    "settings.saving": "保存中…",
    "settings.autoSaved": "已启用自动保存",
    "settings.proxy.title": "代理配置",
    "settings.proxy.description": "配置 /v1/models 劫持与上游合并行为。",
    "settings.proxy.hijackLabel": "劫持 /v1/models",
    "settings.proxy.hijackHint":
      "开启后由当前代理返回预置模型列表，而不是纯透传上游。",
    "settings.proxy.mergeLabel": "实时合并上游模型",
    "settings.proxy.mergeHint":
      "开启后会实时请求上游模型并与预置列表按模型 ID 去重合并。",
    "settings.proxy.mergeDisabledHint": "请先开启劫持，再启用该选项。",
    "settings.proxy.presetModels": "预置模型列表",
    "settings.proxy.enabledCount": "已启用：{{count}} / {{total}}",
    "settings.proxy.noneEnabledHint":
      "当前没有启用任何预置模型；劫持模式下不会返回预置模型。",
    "settings.proxy.fastModeRewriteLabel": "Fast 模式请求改写",
    "settings.proxy.fastModeRewriteHint":
      "仅作用于 POST /v1/responses 与 POST /v1/chat/completions；`requestedServiceTier` 继续表示最终发给上游的值。",
    "settings.proxy.fastModeRewriteDisabled": "关闭",
    "settings.proxy.fastModeRewriteFillMissing": "仅补缺失为 priority",
    "settings.proxy.fastModeRewriteForcePriority": "强制覆盖为 priority",
    "settings.proxy.upstream429RetriesLabel": "上游 429 自动重试",
    "settings.proxy.upstream429RetriesHint":
      "上游返回 429 时先自动重试再决定失败；作用于所有反向代理上游请求，设为 0 可关闭。",
    "settings.proxy.upstream429RetriesDisabled": "关闭（0 次）",
    "settings.proxy.upstream429RetriesOnce": "重试 1 次",
    "settings.proxy.upstream429RetriesMany": "重试 {{count}} 次",
    "settings.forwardProxy.title": "正向代理路由",
    "settings.forwardProxy.description":
      "配置上游请求代理、订阅自动刷新周期与运行期健康指标。",
    "settings.forwardProxy.insertDirectLabel": "插入直连",
    "settings.forwardProxy.insertDirectHint":
      "将直连作为特殊代理加入统一调度，便于兜底与对比。",
    "settings.forwardProxy.proxyUrls": "代理 URL",
    "settings.forwardProxy.proxyUrlsPlaceholder":
      "示例：\nhttp://127.0.0.1:7890\nsocks5://127.0.0.1:1080\nuser:pass@proxy.example.com:8443",
    "settings.forwardProxy.subscriptionUrls": "订阅链接",
    "settings.forwardProxy.subscriptionUrlsPlaceholder":
      "https://example.com/subscription.txt\nhttps://example.com/subscription.base64",
    "settings.forwardProxy.subscriptionInterval": "订阅刷新周期",
    "settings.forwardProxy.interval.1m": "1 分钟",
    "settings.forwardProxy.interval.5m": "5 分钟",
    "settings.forwardProxy.interval.15m": "15 分钟",
    "settings.forwardProxy.interval.1h": "1 小时",
    "settings.forwardProxy.interval.6h": "6 小时",
    "settings.forwardProxy.interval.1d": "1 天",
    "settings.forwardProxy.save": "保存路由配置",
    "settings.forwardProxy.addProxyBatch": "批量导入节点",
    "settings.forwardProxy.addSubscription": "添加订阅",
    "settings.forwardProxy.proxyCount": "已配置节点：{{count}}",
    "settings.forwardProxy.subscriptionCount": "已配置订阅：{{count}}",
    "settings.forwardProxy.nodeItemFallback": "节点 #{{index}}",
    "settings.forwardProxy.subscriptionItemFallback": "订阅 #{{index}}",
    "settings.forwardProxy.remove": "删除",
    "settings.forwardProxy.listEmpty": "暂无条目，请先验证后添加。",
    "settings.forwardProxy.subscriptionListEmpty": "暂无订阅链接。",
    "settings.forwardProxy.supportHint":
      "支持协议：http、https、socks、socks5、socks5h、vmess、vless、trojan、ss（vmess/vless/trojan/ss 由 Xray 转发）",
    "settings.forwardProxy.directLabel": "直连",
    "settings.forwardProxy.modal.description":
      "先输入候选内容并验证可用，验证通过后才可添加。",
    "settings.forwardProxy.modal.step1": "步骤 1：输入节点",
    "settings.forwardProxy.modal.step2": "步骤 2：校验结果",
    "settings.forwardProxy.modal.step1Compact": "输入节点",
    "settings.forwardProxy.modal.step2Compact": "校验结果",
    "settings.forwardProxy.modal.proxyBatchTitle": "批量导入节点",
    "settings.forwardProxy.modal.subscriptionTitle": "添加订阅链接",
    "settings.forwardProxy.modal.proxyBatchInputLabel": "节点信息（每行一个）",
    "settings.forwardProxy.modal.subscriptionInputLabel": "订阅 URL",
    "settings.forwardProxy.modal.proxyBatchPlaceholder":
      "每行一个节点：\nvmess://...\nvless://...\ntrojan://...\nss://...\nhttp://...\nsocks5://...",
    "settings.forwardProxy.modal.subscriptionPlaceholder":
      "https://example.com/subscription.base64",
    "settings.forwardProxy.modal.required": "请先输入内容。",
    "settings.forwardProxy.modal.validating": "正在验证候选项…",
    "settings.forwardProxy.modal.validate": "验证可用性",
    "settings.forwardProxy.modal.add": "添加",
    "settings.forwardProxy.modal.addNode": "添加节点",
    "settings.forwardProxy.modal.retryNode": "重试",
    "settings.forwardProxy.modal.submit": "提交",
    "settings.forwardProxy.modal.submitWithCount": "导入 {{count}} 个节点",
    "settings.forwardProxy.modal.backToStep1": "返回输入",
    "settings.forwardProxy.modal.cancel": "取消",
    "settings.forwardProxy.modal.validateSuccess": "验证通过。",
    "settings.forwardProxy.modal.validateFailed": "验证失败。",
    "settings.forwardProxy.modal.backendUnreachable":
      "无法连接后端服务，请确认 127.0.0.1:8080 已启动。",
    "settings.forwardProxy.modal.backendServerError":
      "后端返回内部错误，请检查后端状态与日志。",
    "settings.forwardProxy.modal.batchValidateSuccess":
      "共 {{count}} 条节点验证通过。",
    "settings.forwardProxy.modal.batchValidateFailed":
      "共 {{total}} 条中有 {{failed}} 条验证失败。",
    "settings.forwardProxy.modal.batchValidateSummary":
      "可用 {{available}} 条，不可用 {{unavailable}} 条。",
    "settings.forwardProxy.modal.batchValidateProgress":
      "正在校验 {{validating}} 条，可用 {{available}} 条，不可用 {{unavailable}} 条。",
    "settings.forwardProxy.modal.normalizedCount": "规范化条目：{{count}}",
    "settings.forwardProxy.modal.normalizedValue": "规范化结果：{{value}}",
    "settings.forwardProxy.modal.probeSummary":
      "解析节点：{{nodes}}，探测延迟：{{latency}}",
    "settings.forwardProxy.modal.unknownNode": "未命名节点",
    "settings.forwardProxy.modal.unknownProtocol": "未知",
    "settings.forwardProxy.modal.rowValidating": "校验中…",
    "settings.forwardProxy.modal.resultIndex": "序号",
    "settings.forwardProxy.modal.resultName": "名称",
    "settings.forwardProxy.modal.resultProtocol": "协议",
    "settings.forwardProxy.modal.resultNode": "节点",
    "settings.forwardProxy.modal.resultStatus": "结果",
    "settings.forwardProxy.modal.resultLatency": "延迟",
    "settings.forwardProxy.modal.resultAction": "操作",
    "settings.forwardProxy.modal.statusAvailable": "可用",
    "settings.forwardProxy.modal.statusUnavailable": "不可用",
    "settings.forwardProxy.modal.statusValidating": "校验中",
    "settings.forwardProxy.modal.statusTimeout": "超时",
    "settings.forwardProxy.modal.roundProgress": "第 {{current}}/{{total}} 轮",
    "settings.forwardProxy.modal.roundResultSuccess":
      "第 {{round}} 轮 成功 {{latency}}",
    "settings.forwardProxy.modal.roundResultTimeout": "第 {{round}} 轮 超时",
    "settings.forwardProxy.modal.roundResultFailed": "第 {{round}} 轮 失败",
    "settings.forwardProxy.table.proxy": "代理",
    "settings.forwardProxy.table.oneMinute": "1 分钟",
    "settings.forwardProxy.table.fifteenMinutes": "15 分钟",
    "settings.forwardProxy.table.oneHour": "1 小时",
    "settings.forwardProxy.table.oneDay": "1 天",
    "settings.forwardProxy.table.sevenDays": "7 天",
    "settings.forwardProxy.table.weight": "权重",
    "settings.forwardProxy.table.successRate": "成功率：{{value}}",
    "settings.forwardProxy.table.avgLatency": "平均延迟：{{value}}",
    "settings.forwardProxy.table.empty": "暂无可用代理条目。",
    "settings.pricing.title": "价格配置",
    "settings.pricing.description": "编辑用于新请求成本估算的模型价格。",
    "settings.pricing.compactNote":
      "compact / 远程压缩请求按命中的模型单价估算成本。",
    "settings.pricing.add": "新增模型",
    "settings.pricing.remove": "删除",
    "settings.pricing.catalogVersion": "价格版本",
    "settings.pricing.columns.model": "模型",
    "settings.pricing.columns.input": "输入 / 1M",
    "settings.pricing.columns.output": "输出 / 1M",
    "settings.pricing.columns.cacheInput": "缓存输入 / 1M",
    "settings.pricing.columns.reasoning": "推理 / 1M",
    "settings.pricing.columns.source": "来源",
    "settings.pricing.columns.actions": "操作",
    "settings.pricing.errors.catalogVersionRequired": "价格版本不能为空。",
    "settings.pricing.errors.modelRequired": "模型 ID 不能为空。",
    "settings.pricing.errors.modelTooLong": "模型 ID 不能超过 128 个字符。",
    "settings.pricing.errors.modelDuplicated": "模型 ID 不能重复。",
    "settings.pricing.errors.numberInvalid": "价格必须是有效数字。",
    "settings.pricing.errors.numberNegative": "价格必须为非负数。",
    "dashboard.section.summaryTitle": "最近 24 小时统计",
    "dashboard.activityOverview.title": "活动总览",
    "dashboard.activityOverview.range24h": "24 小时",
    "dashboard.activityOverview.range7d": "7 日",
    "dashboard.activityOverview.rangeToggleAria": "时间范围切换",
    "dashboard.section.liveRefreshing": "实时刷新",
    "dashboard.section.recentLiveTitle": "最近 {{count}} 条实况",
    "dashboard.today.title": "今日统计信息",
    "dashboard.today.subtitle": "按 {{timezone}} 自然日累计",
    "dashboard.today.dayBadge": "今日",
    "stats.range.lastHour": "最近 1 小时",
    "stats.range.today": "今日",
    "stats.range.lastDay": "最近 1 天",
    "stats.range.thisWeek": "本周",
    "stats.range.lastWeek": "最近 7 天",
    "stats.range.thisMonth": "本月",
    "stats.range.lastMonth": "最近 1 个月",
    "stats.bucket.eachMinute": "每分钟",
    "stats.bucket.each5Minutes": "每 5 分钟",
    "stats.bucket.each15Minutes": "每 15 分钟",
    "stats.bucket.each30Minutes": "每 30 分钟",
    "stats.bucket.eachHour": "每小时",
    "stats.bucket.each6Hours": "每 6 小时",
    "stats.bucket.each12Hours": "每 12 小时",
    "stats.bucket.each24Hours": "每 24 小时",
    "stats.bucket.eachDay": "每天",
    "stats.title": "统计",
    "stats.subtitle": "选择时间范围与聚合粒度",
    "stats.settlementHour": "结算小时",
    "stats.trendTitle": "趋势",
    "stats.successFailureTitle": "成功/失败次数",
    "stats.successFailure.legend.firstByteAvg": "首字耗时均值（ms）",
    "stats.successFailure.tooltip.successRate": "成功率",
    "stats.successFailure.tooltip.firstByteAvg": "首字耗时均值",
    "stats.successFailure.tooltip.firstByteP95": "首字耗时 P95",
    "stats.errors.title": "错误原因分布",
    "stats.errors.scope.label": "失败范围",
    "stats.errors.scope.service": "服务端故障",
    "stats.errors.scope.client": "调用方错误",
    "stats.errors.scope.abort": "客户端中断",
    "stats.errors.scope.all": "全部失败",
    "stats.errors.summary.service": "服务端故障",
    "stats.errors.summary.client": "调用方错误",
    "stats.errors.summary.abort": "客户端中断",
    "stats.errors.summary.actionable": "可行动故障",
    "stats.errors.actionableRate": "可行动失败率：{{rate}}",
    "quota.title": "配额概览",
    "quota.subscription": "订阅：{{name}}",
    "quota.status.active": "正常使用",
    "quota.labels.usageRate": "使用率",
    "quota.labels.used": "已使用",
    "quota.labels.remaining": "剩余额度",
    "quota.labels.nextReset": "下次重置",
    "quota.status.expired": "已到期",
    "quota.status.expireInDays": "到期：剩余{{count}}天",
    "quota.status.expireInHours": "到期：剩余{{count}}小时",
    "quota.status.expireInMinutes": "到期：剩余{{count}}分钟",
    "quota.status.expireAt": "到期：{{time}}",
    "quota.status.expireUnknown": "到期：—",
    "live.summary.current": "当前窗口",
    "live.summary.30m": "30 分钟",
    "live.summary.1h": "1 小时",
    "live.summary.1d": "1 天",
    "live.summary.title": "实时统计",
    "live.chart.title": "实时图表",
    "live.window.label": "窗口大小",
    "live.option.records": "{{count}} 条记录",
    "live.latest.title": "最新记录",
    "live.conversations.title": "Prompt Cache Key 对话",
    "live.conversations.description":
      "按 Prompt Cache Key 聚合对话，可按数量或最近活动时间筛选，累计指标按全历史计算。",
    "live.conversations.selectionLabel": "对话筛选",
    "live.conversations.option.count": "{{count}} 个对话",
    "live.conversations.option.activityHours": "近 {{hours}} 小时活动",
    "live.conversations.empty": "暂无 Prompt Cache Key 对话数据。",
    "live.conversations.implicitFilter.inactiveOutside24h":
      "有 {{count}} 个更新创建的对话因未在近 24 小时活动而未显示；数量模式只统计近 24 小时活跃对话。",
    "live.conversations.implicitFilter.cappedTo50":
      "有 {{count}} 个对话命中了活动时间筛选，但因时间模式最多只展示 50 个对话而未显示。",
    "live.conversations.accountLabel.idFallback": "账号 #{{id}}",
    "live.conversations.table.promptCacheKey": "Prompt Cache Key",
    "live.conversations.table.upstreamAccounts": "上游账号",
    "live.conversations.table.summary": "总计",
    "live.conversations.table.requestCount": "请求数",
    "live.conversations.table.requestCountCompact": "请求",
    "live.conversations.table.totalTokens": "总 Tokens",
    "live.conversations.table.totalTokensCompact": "Token",
    "live.conversations.table.totalCost": "总成本",
    "live.conversations.table.time": "时间",
    "live.conversations.table.createdAt": "创建时间",
    "live.conversations.table.createdAtShort": "创建",
    "live.conversations.table.lastActivityAt": "最后活动",
    "live.conversations.table.lastActivityAtShort": "活动",
    "live.conversations.table.chart24h": "24h Token 累计",
    "live.conversations.table.chartWindow": "{{hours}} 小时 Token 累计",
    "live.conversations.chartAria": "{{hours}} 小时 Token 累计图",
    "live.chart.tooltip.instructions":
      "悬停或点按查看详情；聚焦图表后可用方向键切换数据点。",
    "live.conversations.chart.tooltip.status": "状态",
    "live.conversations.chart.tooltip.requestTokens": "本次 Tokens",
    "live.conversations.chart.tooltip.cumulativeTokens": "累计 Tokens",
    "live.proxy.title": "代理运行态",
    "live.proxy.description":
      "按节点展示多窗口统计（成功率 / 平均延迟）、近 24 小时成功/失败请求量与权重变化趋势。",
    "live.proxy.table.proxy": "代理节点",
    "live.proxy.table.oneMinute": "1 分钟统计",
    "live.proxy.table.fifteenMinutes": "15 分钟统计",
    "live.proxy.table.oneHour": "1 小时统计",
    "live.proxy.table.oneDay": "1 天统计",
    "live.proxy.table.sevenDays": "7 天统计",
    "live.proxy.table.trend24h": "近 24 小时请求量",
    "live.proxy.table.requestTrendAria": "近 24 小时请求量图",
    "live.proxy.table.weightTrend24h": "近 24 小时权重变化",
    "live.proxy.table.weightTrendAria": "近 24 小时权重趋势图",
    "live.proxy.table.requestTooltip.total": "总请求数",
    "live.proxy.table.weightTooltip.samples": "样本",
    "live.proxy.table.weightTooltip.min": "最小",
    "live.proxy.table.weightTooltip.max": "最大",
    "live.proxy.table.weightTooltip.avg": "平均",
    "live.proxy.table.weightTooltip.last": "末值",
    "live.proxy.table.successShort": "成功 {{count}}",
    "live.proxy.table.failureShort": "失败 {{count}}",
    "live.proxy.table.currentWeight": "当前权重 {{value}}",
    "live.proxy.table.empty": "暂无代理统计数据。",
    "records.title": "请求记录",
    "records.subtitle":
      "基于稳定搜索快照分析请求，避免高速新增数据打乱分页结果。",
    "records.filters.title": "筛选",
    "records.filters.description":
      "修改筛选只会更新草稿；点击搜索后才会对齐最新快照。",
    "records.filters.rangePreset": "时间范围",
    "records.filters.rangePreset.today": "今天",
    "records.filters.rangePreset.lastDay": "近 24 小时",
    "records.filters.rangePreset.last7Days": "近 7 天",
    "records.filters.rangePreset.last30Days": "近 30 天",
    "records.filters.rangePreset.custom": "自定义范围",
    "records.filters.from": "开始时间",
    "records.filters.to": "结束时间",
    "records.filters.status": "状态",
    "records.filters.status.all": "全部状态",
    "records.filters.status.success": "成功",
    "records.filters.status.failed": "失败",
    "records.filters.status.running": "运行中",
    "records.filters.status.pending": "排队中",
    "records.filters.any": "全部",
    "records.filters.noMatches": "无匹配结果",
    "records.filters.model": "模型",
    "records.filters.proxy": "代理",
    "records.filters.endpoint": "端点",
    "records.filters.failureClass": "异常类别",
    "records.filters.failureClass.all": "全部类别",
    "records.filters.failureClass.service": "服务端故障",
    "records.filters.failureClass.client": "调用方错误",
    "records.filters.failureClass.abort": "客户端中断",
    "records.filters.upstreamScope": "上游",
    "records.filters.upstreamScope.all": "全部",
    "records.filters.upstreamScope.external": "外部",
    "records.filters.upstreamScope.internal": "内部",
    "records.filters.failureKind": "失败分类",
    "records.filters.promptCacheKey": "Prompt Cache Key",
    "records.filters.requesterIp": "请求方 IP",
    "records.filters.keyword": "关键词",
    "records.filters.minTotalTokens": "最小总 Tokens",
    "records.filters.maxTotalTokens": "最大总 Tokens",
    "records.filters.minTotalMs": "最小总耗时（ms）",
    "records.filters.maxTotalMs": "最大总耗时（ms）",
    "records.filters.search": "搜索",
    "records.filters.searching": "搜索中…",
    "records.filters.clearDraft": "清空草稿",
    "records.focus.label": "记录聚焦",
    "records.focus.token": "Token",
    "records.focus.network": "网络",
    "records.focus.exception": "异常",
    "records.summary.title": "统计",
    "records.summary.description":
      "切换聚焦只会更换统计卡片和列表列，不会刷新当前快照。",
    "records.summary.loadError": "加载统计失败：{{error}}",
    "records.summary.notice.newData": "有 {{count}} 条新数据",
    "records.summary.notice.refreshAction": "加载新数据",
    "records.summary.notice.newDataAria":
      "有 {{count}} 条新数据，点击后会并入当前快照。",
    "records.summary.notice.refreshAria":
      "加载这 {{count}} 条新数据并刷新当前快照。",
    "records.summary.notice.refreshingAria":
      "正在加载这 {{count}} 条新数据并刷新当前快照。",
    "records.summary.token.requests": "请求数",
    "records.summary.token.totalTokens": "总 Tokens",
    "records.summary.token.avgTokens": "单次平均 Tokens",
    "records.summary.token.cacheInput": "缓存输入 Tokens",
    "records.summary.token.totalCost": "总成本",
    "records.summary.network.avgTtfb": "平均 TTFB",
    "records.summary.network.p95Ttfb": "P95 TTFB",
    "records.summary.network.avgTotal": "平均总耗时",
    "records.summary.network.p95Total": "P95 总耗时",
    "records.summary.exception.failures": "失败数",
    "records.summary.exception.service": "服务端故障数",
    "records.summary.exception.client": "调用方错误数",
    "records.summary.exception.abort": "客户端中断数",
    "records.summary.exception.actionable": "可行动故障数",
    "records.list.title": "记录",
    "records.list.description":
      "排序与分页都会锁定在当前快照内，直到你再次点击搜索。",
    "records.list.totalCount": "共 {{count}} 条记录",
    "records.list.pageSize": "分页大小",
    "records.list.sortBy": "排序字段",
    "records.list.sortOrder": "排序方向",
    "records.list.pageLabel": "第 {{page}} / {{totalPages}} 页",
    "records.list.prev": "上一页",
    "records.list.next": "下一页",
    "records.list.sort.occurredAt": "发生时间",
    "records.list.sort.totalTokens": "总 Tokens",
    "records.list.sort.cost": "成本",
    "records.list.sort.totalMs": "总耗时",
    "records.list.sort.ttfb": "TTFB",
    "records.list.sort.status": "状态",
    "records.list.sort.asc": "升序",
    "records.list.sort.desc": "降序",
    "records.table.loadError": "加载记录失败：{{error}}",
    "records.table.loadingAria": "正在加载请求记录",
    "records.table.empty": "当前快照下没有匹配记录。",
    "records.table.details": "记录详情",
    "records.table.showDetails": "展开详情",
    "records.table.hideDetails": "收起详情",
    "records.table.focusTitle": "聚焦摘要",
    "records.table.token.inputCache": "输入 / 缓存",
    "records.table.token.outputReasoning": "输出 / 推理",
    "records.table.token.totalTokens": "总 Tokens",
    "records.table.token.cost": "成本",
    "records.table.network.endpoint": "端点",
    "records.table.network.requesterIp": "请求方 IP",
    "records.table.network.ttfb": "TTFB",
    "records.table.network.firstResponseByteTotal": "首字总耗时",
    "records.table.network.totalMs": "总耗时",
    "records.table.exception.failureKind": "失败分类",
    "records.table.exception.failureClass": "异常类别",
    "records.table.exception.actionable": "可行动",
    "records.table.exception.actionableYes": "可行动",
    "records.table.exception.actionableNo": "不可行动",
    "records.table.exception.error": "错误信息",
    "metric.totalCount": "次数",
    "metric.totalCost": "金额",
    "metric.totalTokens": "Tokens",
    "legend.low": "低",
    "legend.high": "高",
    "calendar.title": "使用活动",
    "calendar.metricsToggleAria": "统计指标切换",
    "calendar.valueSeparator": "：",
    "calendar.timeZoneLabel": "时区",
    "calendar.weekday.sun": "日",
    "calendar.weekday.mon": "一",
    "calendar.weekday.tue": "二",
    "calendar.weekday.wed": "三",
    "calendar.weekday.thu": "四",
    "calendar.weekday.fri": "五",
    "calendar.weekday.sat": "六",
    "calendar.monthLabel": "{{year}}年{{month}}月",
    "heatmap.title": "最近 7 天活动图",
    "heatmap24h.title": "最近 24 小时热点图（10 分钟）",
    "heatmap.metricsToggleAria": "指标切换",
    "heatmap.noData": "暂无数据",
    "table.loadError": "加载记录失败：{{error}}",
    "table.loadingRecordsAria": "正在加载记录",
    "table.noRecords": "暂无记录。",
    "table.column.time": "时间",
    "table.column.model": "模型",
    "table.model.fastPriorityTitle": "Fast 模式（Priority processing）",
    "table.model.fastPriorityAria":
      "当前请求通过 Priority processing 启用 Fast 模式",
    "table.model.fastRequestedOnlyTitle":
      "请求想要 Fast，但实际未命中 Priority processing",
    "table.model.fastRequestedOnlyAria":
      "请求想要 Fast，但实际未命中 Priority processing",
    "table.column.account": "账号",
    "table.column.proxy": "代理",
    "table.column.status": "状态",
    "table.column.inputTokens": "输入",
    "table.column.outputTokens": "输出",
    "table.column.reasoningTokensShort": "推理",
    "table.column.cacheInputTokens": "缓存输入",
    "table.column.reasoningEffort": "推理强度",
    "table.column.totalTokens": "总 Tokens",
    "table.column.costUsd": "成本（美元）",
    "table.column.latency": "用时",
    "table.column.firstByteCompression": "首字耗时 / HTTP 压缩",
    "table.column.firstResponseByteTotalCompression": "首字总耗时 / HTTP 压缩",
    "table.column.totalLatencyShort": "用时",
    "table.column.firstByteLatencyShort": "首字",
    "table.column.firstResponseByteTotalShort": "首字总",
    "table.column.httpCompressionShort": "HTTP",
    "table.latency.firstByteTotal": "首字耗时 / 用时",
    "table.column.error": "错误",
    "table.account.reverseProxy": "反向代理",
    "table.account.poolRoutingPending": "号池路由中",
    "table.account.poolAccountUnavailable": "号池账号未知",
    "table.status.success": "成功",
    "table.status.failed": "失败",
    "table.status.running": "运行中",
    "table.status.pending": "排队中",
    "table.status.unknown": "未知",
    "table.detailsTitle": "请求详情",
    "table.details.invokeId": "调用 ID",
    "table.details.source": "来源",
    "table.details.account": "账号",
    "table.details.proxy": "代理",
    "table.details.endpoint": "端点",
    "table.endpoint.responsesBadge": "Responses",
    "table.endpoint.chatBadge": "Chat",
    "table.endpoint.compactBadge": "远程压缩",
    "table.endpoint.compactHint": "Codex 远程压缩请求",
    "table.details.requesterIp": "请求方 IP",
    "table.details.promptCacheKey": "Prompt Cache Key",
    "table.details.totalLatency": "用时",
    "table.details.firstByteLatency": "首字耗时",
    "table.details.firstResponseByteTotal": "首字总耗时",
    "table.details.httpCompression": "HTTP 压缩算法",
    "table.details.requestedServiceTier": "Requested service tier",
    "table.details.serviceTier": "Service tier",
    "table.details.reasoningEffort": "推理强度",
    "table.details.reasoningTokens": "推理 Tokens",
    "table.details.proxyWeightDelta": "代理权重变化（本次）",
    "table.details.proxyWeightDeltaA11yIncrease": "代理权重上升 {{value}}",
    "table.details.proxyWeightDeltaA11yDecrease": "代理权重下降 {{value}}",
    "table.details.proxyWeightDeltaA11yUnchanged": "代理权重不变（{{value}}）",
    "table.details.failureKind": "失败分类",
    "table.details.streamTerminalEvent": "流终态事件",
    "table.details.upstreamErrorCode": "上游错误码",
    "table.details.upstreamErrorMessage": "上游错误消息",
    "table.details.upstreamRequestId": "上游请求 ID",
    "table.details.poolAttemptCount": "号池尝试次数",
    "table.details.poolDistinctAccountCount": "不同账号数",
    "table.details.poolAttemptTerminalReason": "号池终态原因",
    "table.details.timingsTitle": "阶段耗时",
    "table.details.stage.requestRead": "请求读取",
    "table.details.stage.requestParse": "请求解析",
    "table.details.stage.upstreamConnect": "上游连接",
    "table.details.stage.upstreamFirstByte": "上游首字节",
    "table.details.stage.upstreamStream": "上游流传输",
    "table.details.stage.responseParse": "响应解析",
    "table.details.stage.persistence": "持久化",
    "table.details.stage.total": "总耗时",
    "table.errorDetailsTitle": "错误详情",
    "table.poolAttempts.title": "号池尝试明细",
    "table.poolAttempts.loading": "正在加载号池尝试明细",
    "table.poolAttempts.loadError": "加载号池尝试明细失败：{{error}}",
    "table.poolAttempts.empty": "未找到号池尝试记录，或该明细已经被清理。",
    "table.poolAttempts.notPool": "这条请求没有使用号池路由。",
    "table.poolAttempts.retry": "同账号重试 / 账号序号",
    "table.poolAttempts.httpStatus": "HTTP 状态",
    "table.poolAttempts.failureKind": "失败分类",
    "table.poolAttempts.connectLatency": "连接耗时",
    "table.poolAttempts.firstByteLatency": "首字耗时",
    "table.poolAttempts.streamLatency": "流传输耗时",
    "table.poolAttempts.startedAt": "开始时间",
    "table.poolAttempts.finishedAt": "结束时间",
    "table.poolAttempts.upstreamRequestId": "上游请求 ID",
    "table.poolAttempts.status.success": "成功",
    "table.poolAttempts.status.httpFailure": "HTTP 失败",
    "table.poolAttempts.status.transportFailure": "传输失败",
    "table.poolAttempts.status.budgetExhaustedFinal": "预算耗尽",
    "table.poolAttempts.status.unknown": "未知",
    "table.errorDetailsEmpty": "暂无错误详情。",
    "table.accountDrawer.subtitle": "上游账号",
    "table.accountDrawer.close": "关闭账号详情",
    "table.accountDrawer.fallbackTitle": "上游账号",
    "table.accountDrawer.errorTitle": "加载上游账号详情失败",
    "table.accountDrawer.emptyTitle": "上游账号不可用",
    "table.accountDrawer.emptyBody":
      "该账号可能已被移除，或当前请求已无法关联到可用账号。",
    "table.accountDrawer.openAccountPool": "去号池查看完整详情",
    "table.accountDrawer.healthTitle": "健康信息",
    "table.accountDrawer.healthDescription":
      "展示该上游账号最近同步、令牌与错误状态。",
    "stats.cards.loadError": "加载统计失败：{{error}}",
    "stats.cards.totalCalls": "调用总数",
    "stats.cards.success": "成功",
    "stats.cards.failures": "失败",
    "stats.cards.totalCost": "总成本",
    "stats.cards.totalTokens": "总 Tokens",
    "chart.loading": "加载中",
    "chart.loadingDetailed": "正在加载图表",
    "chart.noDataRange": "所选范围暂无数据。",
    "chart.noDataPoints": "暂无数据点。",
    "chart.totalTokens": "总 Tokens",
    "chart.totalCost": "成本（美元）",
    "chart.totalCount": "次数",
    "unit.calls": "次",
    "quota.status.expired.badge": "已到期",
  },
} as const;

type BaseTranslations = typeof baseTranslations;
type TranslationKey = keyof BaseTranslations["en"];

export const translations: Record<
  Locale,
  Record<TranslationKey, string>
> = baseTranslations;

export type { TranslationKey };

export const FALLBACK_LOCALE: Locale = "en";

export function formatTranslation(
  template: string,
  values?: TranslationValues,
) {
  if (!values) return template;
  return template.replace(/\{\{\s*(\w+)\s*\}\}/g, (_, key: string) => {
    const value = values[key];
    return value === undefined || value === null ? "" : String(value);
  });
}
