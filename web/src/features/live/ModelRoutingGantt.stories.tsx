import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type {
  ModelRoutingLiveAccount,
  ModelRoutingLiveModelGroup,
  ModelRoutingTimelineRecord,
} from "../../lib/api";
import { ModelRoutingGantt } from "./ModelRoutingGantt";

const generatedAt = "2026-08-16T04:00:00.000Z";

const accounts: ModelRoutingLiveAccount[] = [
  {
    accountId: 11,
    accountDisplayName: "API Key #11",
    model: "gpt-5.5",
    state: "available",
    priority: "normal",
    failureCount: 0,
    changedAt: "2026-08-16T03:35:00.000Z",
    lastSeenAt: "2026-08-16T03:59:20.000Z",
  },
  {
    accountId: 12,
    accountDisplayName: "API Key #12",
    model: "gpt-5.5",
    state: "degraded",
    priority: "deprioritized",
    failureCount: 1,
    changedAt: "2026-08-16T03:42:00.000Z",
    lastSeenAt: "2026-08-16T03:58:04.000Z",
  },
  {
    accountId: 13,
    accountDisplayName: "API Key #13",
    model: "gpt-5.5",
    state: "cooling_down",
    priority: "excluded",
    failureCount: 3,
    changedAt: "2026-08-16T03:49:00.000Z",
    lastSeenAt: "2026-08-16T03:49:00.000Z",
    cooldownUntil: "2026-08-16T04:12:00.000Z",
  },
];

const miniAccounts: ModelRoutingLiveAccount[] = [
  {
    accountId: 21,
    accountDisplayName: "API Key #21",
    model: "gpt-5.4-mini",
    state: "degraded",
    priority: "demoted",
    failureCount: 1,
    changedAt: "2026-08-16T03:18:00.000Z",
    lastSeenAt: "2026-08-16T03:57:10.000Z",
  },
  {
    accountId: 22,
    accountDisplayName: "API Key #22",
    model: "gpt-5.4-mini",
    state: "available",
    priority: "normal",
    failureCount: 0,
    changedAt: "2026-08-16T02:54:00.000Z",
    lastSeenAt: "2026-08-16T03:59:40.000Z",
  },
];

const records: ModelRoutingTimelineRecord[] = [
  {
    id: "attempt:11-recovery",
    kind: "attempt",
    occurredAt: "2026-08-16T03:35:00.000Z",
    accountId: 11,
    accountDisplayName: "API Key #11",
    model: "gpt-5.5",
    attemptId: "attempt-11-recovery",
    invokeId: "invoke-11-recovery",
    attemptIndex: 1,
    sameAccountRetryIndex: 0,
    status: "success",
    httpStatus: 200,
    totalLatencyMs: 821,
    reasonCode: "probe_passed",
    modelRouteStateBefore: "cooling_down",
    modelRouteStateAfter: "available",
    modelRoutePriorityBefore: "excluded",
    modelRoutePriorityAfter: "normal",
  },
  {
    id: "attempt:12-degraded",
    kind: "attempt",
    occurredAt: "2026-08-16T03:42:00.000Z",
    accountId: 12,
    accountDisplayName: "API Key #12",
    model: "gpt-5.5",
    attemptId: "attempt-12-degraded",
    invokeId: "invoke-12-degraded",
    attemptIndex: 2,
    sameAccountRetryIndex: 1,
    status: "failed",
    httpStatus: 502,
    totalLatencyMs: 1_237,
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "available",
    modelRouteStateAfter: "degraded",
    modelRoutePriorityBefore: "normal",
    modelRoutePriorityAfter: "demoted",
  },
  {
    id: "event:13-cooling",
    kind: "event",
    occurredAt: "2026-08-16T03:49:00.000Z",
    accountId: 13,
    accountDisplayName: "API Key #13",
    model: "gpt-5.5",
    status: "cooling_down",
    action: "model_route_cooldown",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "degraded",
    modelRouteStateAfter: "cooling_down",
    modelRoutePriorityBefore: "demoted",
    modelRoutePriorityAfter: "excluded",
    modelRouteCooldownUntil: "2026-08-16T04:12:00.000Z",
  },
  {
    id: "event:21-demoted",
    kind: "event",
    occurredAt: "2026-08-16T03:18:00.000Z",
    accountId: 21,
    accountDisplayName: "API Key #21",
    model: "gpt-5.4-mini",
    status: "degraded",
    action: "model_route_degraded",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "available",
    modelRouteStateAfter: "degraded",
    modelRoutePriorityBefore: "normal",
    modelRoutePriorityAfter: "demoted",
  },
];

const groups: ModelRoutingLiveModelGroup[] = [
  { model: "gpt-5.5", accounts },
  { model: "gpt-5.4-mini", accounts: miniAccounts },
];

const meta = {
  title: "Live/ModelRoutingGantt",
  component: ModelRoutingGantt,
  tags: ["autodocs", "test"],
  parameters: { layout: "padded" },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="min-h-screen bg-base-200 p-5 sm:p-8">
          <div className="mx-auto max-w-6xl overflow-hidden rounded-lg border border-base-300 bg-base-100">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  args: {
    groups,
    records,
    generatedAt,
    window: "24h",
    onOpenAccount: fn(),
    onOpenInvocation: fn(),
  },
} satisfies Meta<typeof ModelRoutingGantt>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Operational24Hours: Story = {
  globals: {
    viewport: { value: "desktop1440", isRotated: false },
  },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-routing-gantt")).toBeVisible();
    const legend = within(canvas.getByTestId("model-routing-gantt-legend"));
    await expect(legend.getByText("正常候选")).toBeVisible();
    await expect(legend.getByText("降权候选")).toBeVisible();
    await expect(legend.getByText("已排除")).toBeVisible();
    await expect(legend.getByText("未知")).toBeVisible();
    await expect(legend.getByText("恢复尝试")).toBeVisible();
    await expect(canvas.findByTestId("model-routing-svg-system")).resolves.toBeVisible();
    await expect(canvas.findByTestId("model-routing-model-group-gpt-5.5")).resolves.toBeVisible();
    await expect(
      canvas.findByTestId("model-routing-model-group-gpt-5.4-mini"),
    ).resolves.toBeVisible();
    for (const account of [
      "API Key #11",
      "API Key #12",
      "API Key #13",
      "API Key #21",
      "API Key #22",
    ]) {
      await expect(canvas.findAllByText(account)).resolves.not.toHaveLength(0);
    }
    const ganttHost = canvas.getByTestId("model-routing-gantt-chart-system");
    const ganttContainer = ganttHost.querySelector<HTMLElement>(".gantt-container");
    const laneBar = ganttHost.querySelector<SVGRectElement>(
      '.bar-wrapper[data-id="route-gpt-5x2ex5-11"] .bar',
    );
    const laneLabel = ganttHost.querySelector<SVGTextElement>(
      '.bar-wrapper[data-id="route-gpt-5x2ex5-11"] .bar-label',
    );
    if (!ganttContainer || !laneBar || !laneLabel) {
      throw new Error("routing Gantt layout is incomplete");
    }
    await waitFor(
      () => {
        const containerRect = ganttContainer.getBoundingClientRect();
        const laneRect = laneBar.getBoundingClientRect();
        expect(Math.abs(containerRect.right - laneRect.right)).toBeLessThanOrEqual(4);
      },
      { timeout: 2_000, interval: 50 },
    );
    const containerRect = ganttContainer.getBoundingClientRect();
    const laneRect = laneBar.getBoundingClientRect();
    const labelRect = laneLabel.getBoundingClientRect();
    await expect(laneRect.left - containerRect.left).toBeGreaterThanOrEqual(80);
    await expect(labelRect.right).toBeLessThan(laneRect.left);
    await expect(
      ganttHost.querySelectorAll('[data-routing-priority="normal"]').length,
    ).toBeGreaterThan(0);
    await expect(
      ganttHost.querySelectorAll('[data-routing-priority="demoted"]').length,
    ).toBeGreaterThan(0);
    await expect(
      ganttHost.querySelectorAll('[data-routing-priority="excluded"]').length,
    ).toBeGreaterThan(0);
    await expect(
      canvas.queryByRole("button", { name: /^API Key #12 · 恢复尝试/ }),
    ).not.toBeInTheDocument();
    await expect(
      canvas.findByRole("button", { name: /^API Key #11 · 正常候选 · 可用 ·/ }),
    ).resolves.toBeVisible();

    const attempt = await canvas.findByRole("button", {
      name: /^API Key #11 · 恢复尝试/,
    });
    const attemptRect = attempt.getBoundingClientRect();
    const availableBands = Array.from(
      attempt.parentElement?.querySelectorAll<SVGRectElement>(".model-routing-band--available") ??
        [],
    );
    await expect(
      availableBands.some((band) => {
        const bandRect = band.getBoundingClientRect();
        return (
          attemptRect.right > bandRect.left &&
          attemptRect.left < bandRect.right &&
          attemptRect.bottom > bandRect.top &&
          attemptRect.top < bandRect.bottom
        );
      }),
    ).toBe(false);
    await userEvent.click(attempt);
    await expect(args.onOpenInvocation).toHaveBeenCalledWith("invoke-11-recovery");

    const modelToggle = await canvas.findByRole("button", {
      name: "展开 gpt-5.5 路由记录",
    });
    await userEvent.click(modelToggle);
    const collapseControl = await canvas.findByRole("button", {
      name: "收起 gpt-5.5 路由记录",
    });
    await expect(collapseControl).toHaveAttribute("aria-expanded", "true");
    await expect(collapseControl).toHaveAttribute(
      "aria-controls",
      "model-routing-records-gpt-5x2ex5",
    );
    const modelRecords = await canvas.findByTestId("model-routing-model-records-gpt-5.5");
    const lastSelectedLane = ganttHost.querySelector<SVGRectElement>(
      '.bar-wrapper[data-id="route-gpt-5x2ex5-13"] .bar',
    );
    const nextModel = ganttHost.querySelector<SVGRectElement>(
      '.bar-wrapper[data-id="model-gpt-5x2ex4-mini"] .bar',
    );
    if (!lastSelectedLane || !nextModel) {
      throw new Error("expanded model group boundaries are incomplete");
    }
    const modelRecordsRect = modelRecords.getBoundingClientRect();
    await expect(modelRecordsRect.top).toBeGreaterThanOrEqual(
      lastSelectedLane.getBoundingClientRect().bottom,
    );
    await expect(modelRecordsRect.bottom).toBeLessThanOrEqual(
      nextModel.getBoundingClientRect().top,
    );
    await expect(canvas.getAllByText("gpt-5.5")).toHaveLength(1);
    await expect(within(modelRecords).getByText("重试 1")).toBeVisible();
    await expect(within(modelRecords).getByText("状态事件")).toBeVisible();
    await expect(within(modelRecords).queryByText("gpt-5.4-mini")).not.toBeInTheDocument();
    await userEvent.click(within(modelRecords).getAllByLabelText("展开决策详情")[0]);
    await expect(within(modelRecords).getAllByText("上游服务异常")[0]).toBeVisible();
  },
};

export const Operational24HoursMobile: Story = {
  ...Operational24Hours,
  globals: {
    viewport: { value: "mobile393", isRotated: false },
  },
};

export const Empty: Story = {
  args: {
    groups: [],
    records: [],
  },
};
