import { useEffect, useState } from "react";
import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import type { ForwardProxyBindingNode } from "../lib/api";
import { apiConcurrencyLimitToSliderValue } from "../lib/concurrencyLimit";
import { UpstreamAccountGroupNoteDialog } from "./UpstreamAccountGroupNoteDialog";

type DialogHarnessProps = {
  groupName: string;
  note: string;
  concurrencyLimit?: number;
  existing: boolean;
  busy?: boolean;
  error?: string | null;
  boundProxyKeys?: string[];
  nodeShuntEnabled?: boolean;
  upstream429RetryEnabled?: boolean;
  upstream429MaxRetries?: number;
  availableProxyNodes?: ForwardProxyBindingNode[];
  proxyBindingsCatalogKind?: "ready-empty" | "ready-with-data" | "loading" | "missing" | "deferred";
  proxyBindingsCatalogFreshness?: "fresh" | "stale" | "missing" | "deferred";
};

function buildRequestBuckets(
  seed: number,
  baseline: number,
  failuresEvery: number,
): ForwardProxyBindingNode["last24h"] {
  const start = Date.parse("2026-03-01T00:00:00.000Z");
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString();
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString();
    const successCount = Math.max(
      0,
      Math.round(baseline + Math.sin((index + seed) / 2.4) * (baseline * 0.35)),
    );
    const failureCount =
      index % failuresEvery === 0
        ? Math.max(0, Math.round(1 + ((seed + index) % 3)))
        : 0;
    return {
      bucketStart,
      bucketEnd,
      successCount,
      failureCount,
    };
  });
}

function buildFocusedRequestBuckets(
  points: Record<number, { successCount?: number; failureCount?: number }>,
): ForwardProxyBindingNode["last24h"] {
  const start = Date.parse("2026-03-01T00:00:00.000Z");
  return Array.from({ length: 24 }, (_, index) => {
    const bucketStart = new Date(start + index * 3600_000).toISOString();
    const bucketEnd = new Date(start + (index + 1) * 3600_000).toISOString();
    const point = points[index] ?? {};
    return {
      bucketStart,
      bucketEnd,
      successCount: point.successCount ?? 0,
      failureCount: point.failureCount ?? 0,
    };
  });
}

const directBindingKey = "__direct__";

const defaultForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: directBindingKey,
    source: "direct",
    displayName: "Direct",
    protocolLabel: "DIRECT",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(0, 16, 8),
  },
  {
    key: "fpn_5a7b0c1d2e3f4a10",
    source: "manual",
    displayName: "JP Edge 01",
    protocolLabel: "HTTP",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(1, 18, 7),
  },
  {
    key: "fpn_8b9c0d1e2f3a4b20",
    source: "subscription",
    displayName: "SG Edge 02",
    protocolLabel: "SS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
  {
    key: "fpn_0c1d2e3f4a5b6c40",
    source: "subscription",
    displayName: "US Edge 03",
    protocolLabel: "VLESS",
    penalized: true,
    selectable: true,
    last24h: buildRequestBuckets(9, 10, 4),
  },
  {
    key: "fpn_1d2e3f4a5b6c7d50",
    source: "subscription",
    displayName: "Ivan-la-vless-vision-01KHTAANPS3QM1DB4H8FEWMYEW",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(10, 9, 4),
  },
  {
    key: "fpn_2e3f4a5b6c7d8e60",
    source: "subscription",
    displayName: "Ivan-hkl-ss2022-01KFXRQH56RQ0SJTYQKS68TCYT",
    protocolLabel: "SS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(12, 10, 6),
  },
  {
    key: "fpn_3f4a5b6c7d8e9f70",
    source: "subscription",
    displayName: "Ivan-iijb-vless-vision-01KKNNTZ3DWEENGMWWF3F9NKT1H",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(13, 8, 5),
  },
  {
    key: "fpn_4a5b6c7d8e9f0a80",
    source: "subscription",
    displayName: "Ivan-ap-ss2022-01KHTAB3M332KVBZ0660GJ2PAR",
    protocolLabel: "SS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(14, 9, 5),
  },
  {
    key: "fpn_0d1e2f3a4b5c6d30",
    source: "manual",
    displayName: "Drain Node",
    protocolLabel: "HTTP",
    penalized: true,
    selectable: false,
    last24h: buildRequestBuckets(11, 6, 3),
  },
];

const unicodeForwardProxyNodes: ForwardProxyBindingNode[] = [
  {
    key: "fpb_13579bdf2468ace0",
    source: "subscription",
    displayName: "东京专线 A",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(2, 16, 6),
  },
  {
    key: "fpn_deadbeefcafebabe",
    source: "missing",
    displayName: "历史东京中继",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: false,
    last24h: [],
  },
];

const refreshedDisplayNameNodes: ForwardProxyBindingNode[] = [
  {
    key: "fpb_13579bdf2468ace0",
    aliasKeys: ["fpn_13579bdf2468ace0"],
    source: "subscription",
    displayName: "Tokyo Edge A (Refreshed Label)",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(4, 15, 8),
  },
  {
    key: "fpn_8b9c0d1e2f3a4b20",
    source: "subscription",
    displayName: "SG Edge 02",
    protocolLabel: "SS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
];

const legacyAliasBindingNodes: ForwardProxyBindingNode[] = [
  {
    key: "fpb_canonical_vless_key",
    aliasKeys: ["fpn_legacy_vless_alias"],
    source: "subscription",
    displayName: "Tokyo Edge A",
    protocolLabel: "VLESS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(3, 14, 6),
  },
  {
    key: "fpn_8b9c0d1e2f3a4b20",
    source: "subscription",
    displayName: "SG Edge 02",
    protocolLabel: "SS",
    penalized: false,
    selectable: true,
    last24h: buildRequestBuckets(6, 12, 5),
  },
];

const groupScopedRealTrafficNodes: ForwardProxyBindingNode[] = [
  {
    key: directBindingKey,
    source: "direct",
    displayName: "Direct",
    protocolLabel: "DIRECT",
    penalized: false,
    selectable: true,
    last24h: buildFocusedRequestBuckets({
      18: { successCount: 1 },
      21: { successCount: 1 },
    }),
  },
  {
    key: "fpn_5a7b0c1d2e3f4a10",
    source: "manual",
    displayName: "JP Edge 01",
    protocolLabel: "HTTP",
    penalized: false,
    selectable: true,
    last24h: buildFocusedRequestBuckets({
      17: { successCount: 2 },
      18: { failureCount: 1 },
      20: { successCount: 2 },
      22: { failureCount: 1 },
    }),
  },
  {
    key: "fpn_0c1d2e3f4a5b6c40",
    source: "subscription",
    displayName: "US Edge 03",
    protocolLabel: "VLESS",
    penalized: true,
    selectable: true,
    last24h: buildFocusedRequestBuckets({}),
  },
];

function DialogHarness({
  note: initialNote,
  boundProxyKeys: initialBoundProxyKeys = [],
  nodeShuntEnabled: initialNodeShuntEnabled = false,
  upstream429RetryEnabled: initialUpstream429RetryEnabled = false,
  upstream429MaxRetries: initialUpstream429MaxRetries = 0,
  availableProxyNodes = defaultForwardProxyNodes,
  ...args
}: DialogHarnessProps) {
  const [note, setNote] = useState(initialNote);
  const [concurrencyLimit, setConcurrencyLimit] = useState(
    apiConcurrencyLimitToSliderValue(args.concurrencyLimit ?? 0),
  );
  const [boundProxyKeys, setBoundProxyKeys] = useState(initialBoundProxyKeys);
  const [nodeShuntEnabled, setNodeShuntEnabled] = useState(
    initialNodeShuntEnabled,
  );
  const [upstream429RetryEnabled, setUpstream429RetryEnabled] = useState(
    initialUpstream429RetryEnabled,
  );
  const [upstream429MaxRetries, setUpstream429MaxRetries] = useState(
    initialUpstream429MaxRetries,
  );

  return (
    <div className="min-h-screen bg-base-200 px-6 py-10 text-base-content">
      <div className="mx-auto max-w-3xl rounded-[28px] border border-base-300/70 bg-base-100/80 p-6 shadow-xl backdrop-blur">
        <div className="mb-4 space-y-2">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/45">
            Shared Group Settings
          </p>
          <h1 className="text-2xl font-semibold">
            Upstream account group settings dialog
          </h1>
          <p className="max-w-2xl text-sm leading-6 text-base-content/70">
            This story focuses on the shared group note editor plus hard binding
            for forward proxy nodes.
          </p>
        </div>
        <UpstreamAccountGroupNoteDialog
          open
          {...args}
          note={note}
          concurrencyLimit={concurrencyLimit}
          boundProxyKeys={boundProxyKeys}
          nodeShuntEnabled={nodeShuntEnabled}
          upstream429RetryEnabled={upstream429RetryEnabled}
          upstream429MaxRetries={upstream429MaxRetries}
          availableProxyNodes={availableProxyNodes}
          onNoteChange={setNote}
          onConcurrencyLimitChange={setConcurrencyLimit}
          onBoundProxyKeysChange={setBoundProxyKeys}
          onNodeShuntEnabledChange={setNodeShuntEnabled}
          onUpstream429RetryEnabledChange={(value) => {
            setUpstream429RetryEnabled(value);
            if (value && upstream429MaxRetries <= 0) {
              setUpstream429MaxRetries(1);
            }
          }}
          onUpstream429MaxRetriesChange={setUpstream429MaxRetries}
          onClose={() => undefined}
          onSave={() => undefined}
          title="Edit group settings"
          existingDescription="This group already exists. Saving here updates the shared note and proxy bindings immediately."
          draftDescription="This group is not populated yet. Saving here creates its shared settings in advance."
          noteLabel="Group note"
          notePlaceholder="Capture what this group is for, ownership, and any operational caveats."
          concurrencyLimitLabel="Concurrency limit"
          concurrencyLimitHint="Use 1-30 to cap fresh assignments for this group. The last slider step means unlimited."
          concurrencyLimitCurrentLabel="Current"
          concurrencyLimitUnlimitedLabel="Unlimited"
          cancelLabel="Cancel"
          saveLabel="Save group settings"
          closeLabel="Close dialog"
          existingBadgeLabel="Persisted group"
          draftBadgeLabel="Draft group"
          nodeShuntLabel="Node shunt strategy"
          nodeShuntHint="Each selected node becomes an exclusive slot. Selecting 3 nodes means the group can provide 3 upstream accounts at the same time."
          nodeShuntToggleLabel="Enable exclusive node slots"
          nodeShuntWarning="Enable this strategy only after binding at least one node (including Direct)."
          upstream429RetryLabel="Upstream 429 retry"
          upstream429RetryHint="When enabled, this group keeps the same account and retries after upstream 429 with a random 1-10 second delay."
          upstream429RetryToggleLabel="Retry the same account after upstream 429"
          upstream429RetryCountLabel="Retry count"
          upstream429RetryCountOptions={[
            { value: 1, label: "1 retry" },
            { value: 2, label: "2 retries" },
            { value: 3, label: "3 retries" },
            { value: 4, label: "4 retries" },
            { value: 5, label: "5 retries" },
          ]}
          proxyBindingsLabel="Bound proxy nodes"
          proxyBindingsHint="Leave empty to keep automatic routing. Selected nodes are used as a hard-bound pool for this group."
          proxyBindingsAutomaticLabel="No nodes bound. This group uses automatic routing."
          proxyBindingsLoadingLabel="Loading proxy nodes…"
          proxyBindingsEmptyLabel="No proxy nodes available."
          proxyBindingsMissingLabel="Missing"
          proxyBindingsUnavailableLabel="Unavailable"
          proxyBindingsChartLabel="24h request trend"
          proxyBindingsChartSuccessLabel="Success"
          proxyBindingsChartFailureLabel="Failure"
          proxyBindingsChartEmptyLabel="No 24h data"
          proxyBindingsChartTotalLabel="Total requests"
          proxyBindingsChartAriaLabel="Last 24h request volume chart"
          proxyBindingsChartInteractionHint="Hover or tap for details. Focus the chart and use arrow keys to switch points."
          proxyBindingsChartLocaleTag="en-US"
        />
      </div>
    </div>
  );
}

const meta = {
  title: "Account Pool/Components/Upstream Account Group Settings Dialog",
  component: DialogHarness,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  render: (args) => <DialogHarness {...args} />,
  args: {
    groupName: "production",
    note: "Primary team group for premium traffic and shared routing policies.",
    concurrencyLimit: 6,
    existing: true,
    busy: false,
    error: null,
    boundProxyKeys: [],
    nodeShuntEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableProxyNodes: defaultForwardProxyNodes,
  },
} satisfies Meta<typeof DialogHarness>;

export default meta;

type Story = StoryObj<typeof meta>;

export const AutomaticRouting: Story = {};

export const Upstream429RetryEnabled: Story = {
  args: {
    upstream429RetryEnabled: true,
    upstream429MaxRetries: 3,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Upstream 429 retry/i)).toBeInTheDocument();
    await expect(
      canvas.getByRole("switch", {
        name: /Retry the same account after upstream 429/i,
      }),
    ).toHaveAttribute("aria-checked", "true");
    await expect(canvas.getByText(/3 retries/i)).toBeInTheDocument();
  },
};

export const NodeShuntEnabled: Story = {
  args: {
    boundProxyKeys: [directBindingKey, "fpn_5a7b0c1d2e3f4a10"],
    nodeShuntEnabled: true,
  },
  play: async () => {
    const screen = within(document.body);
    await expect(screen.getByText(/Node shunt strategy/i)).toBeInTheDocument();
    await expect(
      screen.getByRole("switch", { name: /Enable exclusive node slots/i }),
    ).toHaveAttribute("aria-checked", "true");
  },
};

export const Upstream429RetryDisabled: Story = {
  args: {
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByRole("switch", {
        name: /Retry the same account after upstream 429/i,
      }),
    ).toHaveAttribute("aria-checked", "false");
    await expect(
      canvas.getByRole("combobox", { name: /Retry count/i }),
    ).toHaveAttribute("data-disabled");
  },
};

export const HardBoundMultipleNodes: Story = {
  args: {
    boundProxyKeys: [
      directBindingKey,
      "fpn_5a7b0c1d2e3f4a10",
      "fpn_8b9c0d1e2f3a4b20",
    ],
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const chart = await canvas.findByLabelText(
      /JP Edge 01 Last 24h request volume chart/i,
    );
    const firstBar = chart.querySelector('[data-inline-chart-index="0"]');
    if (!(firstBar instanceof HTMLElement)) {
      throw new Error("missing first request trend bar");
    }
    await userEvent.hover(firstBar);
    await expect(
      within(document.body).getByRole("tooltip"),
    ).toBeInTheDocument();
    await expect(
      within(document.body).getByText(/Success/i),
    ).toBeInTheDocument();
    await expect(
      within(document.body).getByText(/Failure/i),
    ).toBeInTheDocument();
    await expect(
      within(document.body).getByText(/Total requests/i),
    ).toBeInTheDocument();
    await expect(canvas.getByText(/^Direct$/i)).toBeInTheDocument();
    await expect(canvas.getByText(/^DIRECT$/i)).toBeInTheDocument();
    await expect(canvas.queryByText(/ss:\/\//i)).not.toBeInTheDocument();
    await expect(
      canvas.getByTestId("proxy-binding-options-scroll-region").className,
    ).toContain("overflow-y-auto");
  },
};

export const GroupScopedRealTraffic: Story = {
  args: {
    groupName: "prod",
    note: "Only this group's real pool attempts are shown here; global probe noise stays out of the 24H totals.",
    boundProxyKeys: [
      directBindingKey,
      "fpn_5a7b0c1d2e3f4a10",
      "fpn_0c1d2e3f4a5b6c40",
    ],
    availableProxyNodes: groupScopedRealTrafficNodes,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/^JP Edge 01$/i)).toBeInTheDocument();
    await expect(canvas.getAllByText(/^2$/).length).toBeGreaterThan(0);
    await expect(canvas.getByText(/^Direct$/i)).toBeInTheDocument();
    await expect(canvas.getByText(/^US Edge 03$/i)).toBeInTheDocument();
  },
};

export const NonAsciiBindings: Story = {
  args: {
    groupName: "apac-premium",
    note: "Stable keys survive refreshes while operators still see localized display names.",
    boundProxyKeys: ["fpn_13579bdf2468ace0", "fpn_deadbeefcafebabe"],
    availableProxyNodes: unicodeForwardProxyNodes,
  },
};

export const MissingOrUnavailableBindings: Story = {
  args: {
    groupName: "overflow",
    note: "Legacy overflow group with one restored stale binding and one currently unavailable node.",
    boundProxyKeys: ["fpn_0d1e2f3a4b5c6d30", "fpn_deadbeefcafebabe"],
    availableProxyNodes: [
      ...defaultForwardProxyNodes,
      unicodeForwardProxyNodes[1],
    ],
  },
};

export const LoadingProxyCatalog: Story = {
  args: {
    groupName: "warming-up",
    note: "This dialog is still waiting for the shared proxy catalog to hydrate.",
    availableProxyNodes: [],
    proxyBindingsCatalogKind: "loading",
    proxyBindingsCatalogFreshness: "missing",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Loading proxy nodes/i)).toBeInTheDocument();
    await expect(
      canvas.queryByText(/No proxy nodes available/i),
    ).not.toBeInTheDocument();
  },
};

export const SettingsSaveSyncRefresh: Story = {
  render: (args) => {
    function SettingsSyncRefreshHarness() {
      const [catalog, setCatalog] = useState<ForwardProxyBindingNode[]>([]);
      const [catalogKind, setCatalogKind] = useState<
        "ready-empty" | "ready-with-data" | "loading" | "missing" | "deferred"
      >("missing");
      const [freshness, setFreshness] = useState<
        "fresh" | "stale" | "missing" | "deferred"
      >("stale");

      useEffect(() => {
        const timer = window.setTimeout(() => {
          setCatalog([
            defaultForwardProxyNodes.find((node) => node.key === "fpn_5a7b0c1d2e3f4a10")!,
            defaultForwardProxyNodes.find((node) => node.key === "fpn_8b9c0d1e2f3a4b20")!,
          ]);
          setCatalogKind("ready-with-data");
          setFreshness("fresh");
        }, 450);
        return () => window.clearTimeout(timer);
      }, []);

      return (
        <DialogHarness
          {...args}
          groupName="settings-sync"
          note="Starts stale and empty, then repaints in place once the refreshed proxy catalog lands."
          availableProxyNodes={catalog}
          proxyBindingsCatalogKind={catalogKind}
          proxyBindingsCatalogFreshness={freshness}
        />
      );
    }

    return <SettingsSyncRefreshHarness />;
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Loading proxy nodes/i)).toBeInTheDocument();
    await expect(
      canvas.queryByText(/No proxy nodes available/i),
    ).not.toBeInTheDocument();
    await expect(await canvas.findByText(/JP Edge 01/i)).toBeInTheDocument();
  },
};

export const UnavailableOnlyBindingsBlockSave: Story = {
  args: {
    groupName: "drain-only",
    note: "This group currently only references unavailable bindings and must not save until one selectable node is chosen.",
    boundProxyKeys: ["fpn_0d1e2f3a4b5c6d30"],
    availableProxyNodes: defaultForwardProxyNodes.filter(
      (node) => node.key === "fpn_0d1e2f3a4b5c6d30",
    ),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByText(
        /select at least one available proxy node or clear bindings before saving\./i,
      ),
    ).toBeInTheDocument();
    await expect(
      canvas.getByRole("button", { name: /save group settings/i }),
    ).toBeDisabled();
    await expect(canvas.getByText(/^Unavailable$/i)).toBeInTheDocument();
  },
};

export const RefreshedDisplayNameStableBinding: Story = {
  args: {
    groupName: "refresh-proof",
    note: "The stable binding key remains selected after the subscription remark changes.",
    boundProxyKeys: ["fpb_13579bdf2468ace0"],
    availableProxyNodes: refreshedDisplayNameNodes,
  },
};

export const LegacyAliasBindingsRemainSaveable: Story = {
  args: {
    groupName: "legacy-alias",
    note: "Groups saved with legacy VLESS aliases still resolve to the current stable node and can be re-saved.",
    boundProxyKeys: ["fpn_legacy_vless_alias"],
    availableProxyNodes: legacyAliasBindingNodes,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/^Tokyo Edge A$/i)).toBeInTheDocument();
    await expect(
      canvas.queryByText(
        /select at least one available proxy node or clear bindings before saving\./i,
      ),
    ).not.toBeInTheDocument();
    await expect(
      canvas.getByRole("button", { name: /save group settings/i }),
    ).toBeEnabled();
  },
};

export const UnlimitedDraft: Story = {
  args: {
    groupName: "bursting",
    note: "",
    concurrencyLimit: 0,
    existing: false,
    boundProxyKeys: [],
  },
};
