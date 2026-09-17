import { expect, userEvent, waitFor, within } from "storybook/test";
import { STORYBOOK_TTFT_RESPONSE_DURATION_RECORDS } from "../records/invocationRecordsStoryFixtures";
import {
  compactLatencyFormattingRecords,
  defaultArgs,
  endpointBadgeRecords,
  fastIndicatorRecords,
  InvocationTableSharedDrawerPreview,
  inFlightResponseDurationUnavailableRecords,
  legacyModelOnlyRecords,
  missingWindowDrawerRecords,
  modelRoutingMismatchRecords,
  PoolAttemptDetailLifecyclePreview,
  poolRoutingAccountStateRecords,
  Recent20StreamingPreview,
  RunningInvocationLifecyclePreview,
  reasoningEffortRecords,
  type Story,
  meta as storyMeta,
} from "./InvocationTable.stories.support";

const meta = { ...storyMeta };
export default meta;

export const Default: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          "Reference state with pool-routed and reverse-proxy invocations. Verify the `账号 / 代理` split, the dedicated elapsed/compression column, and the reasoning-token breakdown in the output summary.",
      },
    },
  },
};

export const CardInteraction: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          "Interaction coverage for the shared card contract: the whole card and keyboard activation toggle the existing workflow detail panel, while the nested account action remains a separate navigation affordance.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const card = canvas.getAllByTestId("invocation-card")[0];
    expect(card).toHaveAttribute("data-expanded", "false");

    card.focus();
    await userEvent.keyboard("{Enter}", { delay: 0 });
    expect(card).toHaveAttribute("data-expanded", "true");

    await userEvent.keyboard(" ", { delay: 0 });
    expect(card).toHaveAttribute("data-expanded", "false");

    const accountButton = canvas.queryByRole("button", { name: /Codex Team Alpha/i });
    if (accountButton) {
      await userEvent.click(accountButton);
      expect(card).toHaveAttribute("data-expanded", "false");
    }
  },
};

export const ModelRoutingMismatch: Story = {
  args: {
    records: modelRoutingMismatchRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Shows the routed-model state: the primary badge text follows the actual response model, while a compare icon appears only when the normalized request and response models differ.",
      },
    },
  },
};

export const LegacyModelOnly: Story = {
  args: {
    records: legacyModelOnlyRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Legacy fallback state: rows with only the historical `model` field keep that value as the response-model display, while request model remains unavailable in the expanded detail view.",
      },
    },
  },
};

export const PoolRoutingAccountStates: Story = {
  args: {
    records: poolRoutingAccountStateRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Compares pool routing account attribution while a request is still running, the no-account fallback, and the terminal account state. Only the running concrete account should use the breathing primary text treatment.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const runningAccount = await canvas.findByRole("button", { name: "Codex Team Alpha" });
    await expect(runningAccount.className).toContain("invocation-account-routing-in-progress");
    await expect(canvas.getByText(/号池路由中|pool routing/i)).toBeInTheDocument();
    const terminalAccount = await canvas.findByRole("button", { name: "Codex Team Beta" });
    await expect(terminalAccount.className).not.toContain("invocation-account-routing-in-progress");
  },
};

export const TransportBadgeMixed: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          "Mixed transport state: only the WebSocket invocation shows the compact `WS` badge beside the model name, while HTTP/legacy records remain unbadged.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = canvasElement.querySelectorAll('[data-testid="invocation-transport-badge"]');
    expect(badges.length).toBeGreaterThanOrEqual(1);
    expect(
      Array.from(badges).every(
        (badge) => badge.querySelector('[aria-hidden="true"]')?.textContent === "WS",
      ),
    ).toBe(true);
  },
};

export const RunningLifecycleSimulation: Story = {
  args: defaultArgs,
  render: () => <RunningInvocationLifecyclePreview />,

  parameters: {
    docs: {
      description: {
        story:
          "Mock story for the new live-running experience: the row appears immediately as `running`, later receives TTFB plus separate request/response HTTP compression diagnostics, and finally swaps in the terminal persisted record without duplicating the row. The terminal step intentionally switches from a negative temporary id to a positive persisted id while keeping the same `invokeId + occurredAt` stable key.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Storybook Live Running Demo/i)).toBeInTheDocument();
    await expect(canvas.getByText(/运行中|running/i)).toBeInTheDocument();

    const toggleButtons = await canvas.findAllByRole("button", { name: /展开详情|show details/i });
    await userEvent.click(toggleButtons[0]);

    await waitFor(
      async () => {
        await expect(
          canvas.getByText(/HTTP 响应压缩|HTTP response compression/i),
        ).toBeInTheDocument();
        await expect(canvas.getByText(/gzip/i)).toBeInTheDocument();
      },
      { timeout: 4000 },
    );

    await waitFor(
      async () => {
        await expect(canvas.getByText(/成功|success/i)).toBeInTheDocument();
      },
      { timeout: 5000 },
    );
  },
};

export const PoolAttemptDetailLifecycle: Story = {
  args: defaultArgs,
  render: () => <PoolAttemptDetailLifecyclePreview />,
  parameters: {
    docs: {
      description: {
        story:
          "Demonstrates the pool-attempt detail lifecycle from this task: opening the expanded panel immediately shows the started `pending` attempt, then the same row refreshes in place to `success` once the terminal snapshot lands. Use it to verify there is no empty-state gap, no duplicate attempt row, and no regression back to stale pending data.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/Storybook Pool Attempt Lifecycle/i)).toBeInTheDocument();

    const toggleButtons = await canvas.findAllByRole("button", { name: /展开详情|show details/i });
    await userEvent.click(toggleButtons[0]);

    await waitFor(
      async () => {
        await expect(canvas.getByText(/号池尝试明细|pool attempt details/i)).toBeInTheDocument();
        await expect(canvas.getByText(/进行中|pending|running/i)).toBeInTheDocument();
        await expect(canvas.getByText(/Storybook Dallas Egress/i)).toBeInTheDocument();
      },
      { timeout: 3000 },
    );

    await waitFor(
      async () => {
        await expect(canvas.getByText(/成功|success/i)).toBeInTheDocument();
        await expect(canvas.getByText(/req_storybook_pool_attempt_final/i)).toBeInTheDocument();
      },
      { timeout: 4000 },
    );
  },
};

export const Recent20StreamingSimulation: Story = {
  args: defaultArgs,
  render: () => <Recent20StreamingPreview />,
  parameters: {
    docs: {
      description: {
        story:
          "Simulates the “最近 20 条实况” surface with a continuously moving stream: new requests keep appearing at the top, several rows remain in `running`, and each request finishes after a different delay so the table mixes success, failure, and in-flight unavailable timings at the same time. New arrivals are randomized between 3 and 10 seconds to better match a real monitoring feed, and the canvas stays capped near 20 visible rows to mirror the real dashboard/live summary view.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await waitFor(
      async () => {
        const rowCount = canvasElement.querySelectorAll('[data-testid="invocation-card"]').length;
        expect(rowCount).toBeGreaterThanOrEqual(12);
      },
      { timeout: 3000 },
    );

    await waitFor(
      async () => {
        await expect(canvas.getByText(/运行中|running/i)).toBeInTheDocument();
      },
      { timeout: 5000 },
    );

    await waitFor(
      async () => {
        const statusText = canvasElement.textContent ?? "";
        expect(/成功|success/i.test(statusText)).toBe(true);
        expect(/失败|failed/i.test(statusText)).toBe(true);
      },
      { timeout: 7000 },
    );
  },
};

export const ExpandedDetails: Story = {
  args: defaultArgs,
  parameters: {
    docs: {
      description: {
        story:
          "Auto-expands the first invocation so you can review the default open detail layout, including account attribution, latency fields, and timing-stage breakdown without needing manual interaction.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const toggleButtons = await canvas.findAllByRole("button", { name: /展开详情|show details/i });
    await userEvent.click(toggleButtons[0]);
    await expect(canvas.getByText(/请求详情|request details/i)).toBeInTheDocument();
    await expect(canvas.getByText(/HTTP 响应压缩|HTTP response compression/i)).toBeInTheDocument();
  },
};

export const TtftAndResponseDuration: Story = {
  args: {
    records: STORYBOOK_TTFT_RESPONSE_DURATION_RECORDS,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Focused verification state for record latency semantics. The first row shows compact `TTFT = 9.4 s` and independent `响应耗时 = 10.1 s`; expanded diagnostics retain `上游首字节 = 0.0 ms` without using it as either value.",
      },
    },
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByTestId("invocation-card-ttft")[0]).toHaveTextContent("9.4 s");
    await expect(canvas.getAllByTestId("invocation-card-response")[0]).toHaveTextContent("10.1 s");
  },
};

export const CompactLatencyFormatting: Story = {
  args: {
    records: compactLatencyFormattingRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Compact latency formatting keeps at most one decimal below 100 seconds and removes the decimal at 100 seconds or above.",
      },
    },
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const ttftValues = await canvas.findAllByTestId("invocation-card-ttft");
    const responseValues = await canvas.findAllByTestId("invocation-card-response");
    await expect(ttftValues[0]).toHaveTextContent("1.2 s");
    await expect(responseValues[0]).toHaveTextContent("7.9 s");
    await expect(ttftValues[1]).toHaveTextContent("100 s");
    await expect(responseValues[1]).toHaveTextContent("124 s");
  },
};

export const InFlightResponseDurationUnavailable: Story = {
  args: {
    records: inFlightResponseDurationUnavailableRecords,
    isLoading: false,
    error: null,
  },
  globals: { themeMode: "dark" },
  parameters: {
    docs: {
      description: {
        story:
          "Focused responding state: `firstTokenMs` is already measured and shown as TTFT, while the unfinished `tUpstreamStreamMs` remains `--`. The UI never substitutes elapsed wall-clock time, and TTFT uses the shared success green.",
      },
    },
  },
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvasElement.ownerDocument.documentElement.getAttribute("data-theme")).toBe(
      "vibe-dark",
    );
    await expect(getComputedStyle(canvas.getByTestId("invocation-card")).backgroundColor).toContain(
      "0.234",
    );
    const ttft = canvas.getByTestId("invocation-card-ttft");
    await expect(ttft).toHaveTextContent("0.7 s");
    await expect(ttft.className).toContain("text-success");
    await expect(canvas.getByTestId("invocation-card-response")).toHaveTextContent("--");
  },
};

export const AccountDrawer: Story = {
  args: defaultArgs,
  render: (args) => <InvocationTableSharedDrawerPreview {...args} />,
  parameters: {
    docs: {
      description: {
        story:
          "Clicks the first pool account badge and verifies that the shared upstream-account drawer opens in-place with the same detail surface used by the account-pool page.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);
    await userEvent.click(await canvas.findByRole("button", { name: "Codex Team Alpha" }));
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Team Alpha/i });
    await expect(within(dialog).getByRole("tab", { name: /概览|overview/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(
      within(dialog).getByText(/最近成功同步|last successful sync/i),
    ).toBeInTheDocument();
    await expect(within(dialog).getByText(/22 \/ 100/i)).toBeInTheDocument();
    await userEvent.click(within(dialog).getByRole("tab", { name: /健康|health/i }));
    await expect(
      within(dialog).getByText(
        /Two upstream 429 responses were observed during the latest compact capability probe\./i,
      ),
    ).toBeInTheDocument();
  },
};

export const MissingWindowPlaceholders: Story = {
  render: (args) => <InvocationTableSharedDrawerPreview {...args} />,
  args: {
    records: missingWindowDrawerRecords,
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Opens the shared upstream-account drawer for an account whose weekly quota window is missing, so the overview usage cards can be reviewed in placeholder mode without conflating it with a real `0%` snapshot.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);
    await userEvent.click(
      await canvas.findByRole("button", { name: "Team key - missing weekly limit" }),
    );
    const dialog = await documentScope.findByRole("dialog", {
      name: /Team key - missing weekly limit/i,
    });
    await expect(within(dialog).getByText(/18 requests/i)).toBeInTheDocument();
    expect(within(dialog).getAllByText("-").length).toBeGreaterThanOrEqual(4);
    await expect(
      within(dialog).queryByText(/还没有额度历史|No quota history yet/i),
    ).not.toBeInTheDocument();
  },
};

export const SharedDrawerUrlState: Story = {
  args: defaultArgs,
  render: (args) => <InvocationTableSharedDrawerPreview {...args} />,
  parameters: {
    docs: {
      description: {
        story:
          "Opens the shared upstream-account drawer from the monitoring table and verifies that the current page URL state now carries the selected `upstreamAccountId` without navigating away.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);
    await userEvent.click(await canvas.findByRole("button", { name: "Codex Team Alpha" }));
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Team Alpha/i });
    await expect(within(dialog).getByText(/账号 ID|ChatGPT account id/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/org_alpha/i)).toBeInTheDocument();
    await expect(documentScope.getByTestId("invocation-route-state")).toHaveTextContent(
      "/dashboard?upstreamAccountId=21",
    );
  },
};

export const FastIndicatorStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Covers the fast indicator matrix: effective priority, API Keys requested-tier priority where the response tier is only `default`, requested-only fallback, requested priority with missing response tier, effective priority despite non-priority request, and a flex request with no lightning icon.",
      },
    },
  },
  args: {
    records: fastIndicatorRecords,
    isLoading: false,
    error: null,
  },
};

export const ApiKeysRequestedTierPriority: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Stable API Keys example: the request asked for `priority`, the upstream response only reported `default`, but billing resolves from the requested-tier strategy and the lightning badge stays effective.",
      },
    },
  },
  args: {
    records: [
      {
        id: 2101,
        invokeId: "inv_api_keys_requested_tier_priority",
        occurredAt: "2026-04-06T06:28:02Z",
        createdAt: "2026-04-06T06:28:02Z",
        source: "proxy",
        routeMode: "pool",
        upstreamAccountId: 2568,
        upstreamAccountName: "API Keys Pool",
        proxyDisplayName: "api-keys-requested-tier",
        endpoint: "/v1/responses",
        model: "gpt-5.4",
        status: "success",
        requesterIp: "203.0.113.88",
        requestedServiceTier: "priority",
        serviceTier: "default",
        billingServiceTier: "priority",
        totalTokens: 4096,
        cost: 0.1024,
        priceVersion: "openai-standard-2026-02-23@requested-tier",
        tUpstreamTtfbMs: 188.6,
        tTotalMs: 890.1,
      },
    ],
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);
    const fastBadge = canvasElement.querySelector('[data-fast-state="effective"]');
    expect(fastBadge).not.toBeNull();
    await userEvent.click(await canvas.findByRole("button", { name: /展开详情|show details/i }));
    await expect(documentScope.getByText(/^Requested service tier$/i)).toBeInTheDocument();
    await expect(documentScope.getByText(/^Service tier$/i)).toBeInTheDocument();
    await expect(documentScope.getByText(/^Billing service tier$/i)).toBeInTheDocument();
  },
};

export const EndpointBadgeStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Shows the shared endpoint-summary matrix: recognized `responses/chat/compact`, image-family endpoints (`image/gen`, `image/edit`, `image`), and both request-side and response-side remote compaction V2 states render as chips, while an unknown long non-image endpoint stays on the raw monospace fallback path. Expanded details still retain the original endpoint text.",
      },
    },
  },
  args: {
    records: endpointBadgeRecords,
    isLoading: false,
    error: null,
  },
};

export const ReasoningEffortStates: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Matrix story for visually checking every reasoning effort state the table may show: `none`, `minimal`, `low`, `medium`, `high`, `xhigh`, `max`, `ultra`, missing (`—`), and an unknown raw string. The intended color ladder is neutral -> cool -> primary -> warning -> error, with unknown values rendered as dashed neutral badges.",
      },
    },
  },
  args: {
    records: reasoningEffortRecords,
    isLoading: false,
    error: null,
  },
};

export const Empty: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Empty state used when the request succeeds but no invocations match the current filters.",
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: null,
  },
};

export const Loading: Story = {
  parameters: {
    docs: {
      description: {
        story: "Loading placeholder used while invocation records are being fetched or refreshed.",
      },
    },
  },
  args: {
    records: [],
    isLoading: true,
    error: null,
  },
};

export const LoadError: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Error banner state used when the invocation request fails and the user needs retry context.",
      },
    },
  },
  args: {
    records: [],
    isLoading: false,
    error: "Request failed: 500 Internal Server Error",
  },
};
