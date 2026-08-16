import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, fn, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingLiveAccount, ModelRoutingTimelineRecord } from "../../lib/api";
import { ModelRoutingGantt } from "./ModelRoutingGantt";

const generatedAt = "2026-08-16T04:00:00.000Z";

const accounts: ModelRoutingLiveAccount[] = [
  {
    accountId: 11,
    accountDisplayName: "Ciii",
    model: "gpt-5.5",
    state: "available",
    priority: "normal",
    failureCount: 0,
    changedAt: "2026-08-16T03:35:00.000Z",
    lastSeenAt: "2026-08-16T03:59:20.000Z",
  },
  {
    accountId: 12,
    accountDisplayName: "Ciii2",
    model: "gpt-5.5",
    state: "degraded",
    priority: "deprioritized",
    failureCount: 1,
    changedAt: "2026-08-16T03:42:00.000Z",
    lastSeenAt: "2026-08-16T03:58:04.000Z",
  },
  {
    accountId: 13,
    accountDisplayName: "Ciii-recovery",
    model: "gpt-5.5",
    state: "cooling_down",
    priority: "excluded",
    failureCount: 3,
    changedAt: "2026-08-16T03:49:00.000Z",
    lastSeenAt: "2026-08-16T03:49:00.000Z",
    cooldownUntil: "2026-08-16T04:12:00.000Z",
  },
];

const records: ModelRoutingTimelineRecord[] = [
  {
    id: "attempt:ciii-recovery",
    kind: "attempt",
    occurredAt: "2026-08-16T03:35:00.000Z",
    accountId: 11,
    accountDisplayName: "Ciii",
    model: "gpt-5.5",
    attemptId: "attempt-ciii-recovery",
    invokeId: "invoke-ciii-recovery",
    attemptIndex: 1,
    sameAccountRetryIndex: 0,
    status: "success",
    httpStatus: 200,
    totalLatencyMs: 821,
    reasonCode: "probe_passed",
    modelRouteStateBefore: "cooling_down",
    modelRouteStateAfter: "available",
  },
  {
    id: "attempt:ciii2-degraded",
    kind: "attempt",
    occurredAt: "2026-08-16T03:42:00.000Z",
    accountId: 12,
    accountDisplayName: "Ciii2",
    model: "gpt-5.5",
    attemptId: "attempt-ciii2-degraded",
    invokeId: "invoke-ciii2-degraded",
    attemptIndex: 2,
    sameAccountRetryIndex: 1,
    status: "failed",
    httpStatus: 502,
    totalLatencyMs: 1_237,
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "available",
    modelRouteStateAfter: "degraded",
  },
  {
    id: "event:ciii2-cooling",
    kind: "event",
    occurredAt: "2026-08-16T03:49:00.000Z",
    accountId: 13,
    accountDisplayName: "Ciii-recovery",
    model: "gpt-5.5",
    status: "cooling_down",
    action: "model_route_cooldown",
    reasonCode: "upstream_http_5xx",
    modelRouteStateBefore: "degraded",
    modelRouteStateAfter: "cooling_down",
    modelRouteCooldownUntil: "2026-08-16T04:12:00.000Z",
  },
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
    model: "gpt-5.5",
    accounts,
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
    await expect(canvas.getByTestId("model-routing-gantt-gpt-5.5")).toBeVisible();
    await expect(canvas.getByText("可用")).toBeVisible();
    await expect(canvas.getByText("降权")).toBeVisible();
    await expect(canvas.getByText("冷却中")).toBeVisible();
    await expect(canvas.getByText("未知")).toBeVisible();
    await expect(canvas.getByText("请求尝试")).toBeVisible();

    const attempt = canvas.getByRole("button", { name: /^Ciii · 请求尝试/ });
    await userEvent.click(attempt);
    await expect(args.onOpenInvocation).toHaveBeenCalledWith("invoke-ciii-recovery");
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
    accounts: [],
    records: [],
  },
};
