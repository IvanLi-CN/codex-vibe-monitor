import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ComponentType, useInsertionEffect } from "react";
import { expect, fn, userEvent, waitFor, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelRoutingState } from "../../lib/api";
import { ModelRoutingHealthPanel } from "./ModelRoutingHealthPanel";

const now = new Date("2026-07-24T08:00:00.000Z").toISOString();

const states: ModelRoutingState[] = [
  {
    model: "gpt-5.5",
    state: "available",
    priority: "normal",
    failureCount: 0,
    changedAt: now,
    lastSeenAt: now,
  },
  {
    model: "gpt-5.4-mini",
    state: "degraded",
    priority: "demoted",
    failureCount: 3,
    changedAt: now,
    lastSeenAt: now,
    lastFailureAt: now,
    lastFailureKind: "model_unavailable",
    lastFailureMessage: "The requested model is temporarily unavailable upstream.",
  },
  {
    model: "o4-mini",
    state: "cooling_down",
    priority: "excluded",
    failureCount: 5,
    changedAt: now,
    lastSeenAt: now,
    lastFailureAt: now,
    lastFailureKind: "upstream_http_429_quota_exhausted",
    lastFailureMessage: "Model-specific quota exhausted.",
    cooldownUntil: new Date("2026-07-24T08:00:45.000Z").toISOString(),
  },
];

function ModelRoutingHistoryFetchMock() {
  useInsertionEffect(() => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      if (url.includes("/api/pool/upstream-accounts/55/model-routing-events")) {
        return new Response(
          JSON.stringify({
            items: [
              {
                id: "attempt:route-recovery",
                kind: "attempt",
                occurredAt: "2026-07-24T07:59:12.000Z",
                accountId: 55,
                accountDisplayName: "Ciii",
                model: "gpt-5.5",
                attemptId: "attempt-route-recovery",
                invokeId: "invoke-route-recovery",
                attemptIndex: 3,
                sameAccountRetryIndex: 1,
                status: "success",
                httpStatus: 200,
                totalLatencyMs: 821,
                reasonCode: "probe_passed",
                modelRouteStateBefore: "cooling_down",
                modelRouteStateAfter: "available",
              },
              {
                id: "event:route-cooling",
                kind: "event",
                occurredAt: "2026-07-24T07:57:01.000Z",
                accountId: 55,
                accountDisplayName: "Ciii",
                model: "gpt-5.5",
                action: "model_route_cooling_started",
                reasonCode: "cache_hit_rate_low",
                modelRouteStateBefore: "degraded",
                modelRouteStateAfter: "cooling_down",
              },
            ],
            nextCursor: "history-page-2",
          }),
          { status: 200, headers: { "Content-Type": "application/json" } },
        );
      }
      return originalFetch(input, init);
    };
    return () => {
      globalThis.fetch = originalFetch;
    };
  }, []);

  return null;
}

const withModelRoutingHistoryFetchMock = (Story: ComponentType) => (
  <>
    <ModelRoutingHistoryFetchMock />
    <Story />
  </>
);

const meta = {
  title: "Account Pool/ModelRoutingHealthPanel",
  component: ModelRoutingHealthPanel,
  tags: ["autodocs", "test"],
  parameters: {
    layout: "fullscreen",
    a11y: {
      options: { rules: { "color-contrast": { enabled: true } } },
      config: { rules: [{ id: "color-contrast", enabled: true }] },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <div className="bg-neutral p-8 text-neutral-content sm:p-12">
          <div className="mx-auto max-w-[1440px]">
            <Story />
          </div>
        </div>
      </I18nProvider>
    ),
  ],
  args: { accountId: 55, states, writesEnabled: true, onReset: fn() },
} satisfies Meta<typeof ModelRoutingHealthPanel>;

export default meta;
type Story = StoryObj<typeof meta>;

export const MixedStates: Story = {
  globals: {
    viewport: { value: "desktop1440", isRotated: false },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("gpt-5.5", { exact: true })).toBeVisible();
    await expect(canvas.queryByText("模型不可用", { exact: true })).not.toBeInTheDocument();
    await expect(canvas.getAllByLabelText("展开模型路由历史")[0]).toBeVisible();
  },
};

export const MixedStatesMobile: Story = {
  ...MixedStates,
  globals: {
    viewport: { value: "mobile393", isRotated: false },
  },
};

export const ExpandedHistory: Story = {
  args: { initialExpandedModel: "gpt-5.5" },
  decorators: [withModelRoutingHistoryFetchMock],
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(() => expect(canvas.getByTitle("probe_passed")).toBeVisible());
    await expect(canvas.getByText("加载更早事件")).toBeVisible();
  },
};

export const Empty: Story = {
  args: { states: [] },
};

export const ResetCoolingModel: Story = {
  args: { states, writesEnabled: true, onReset: fn() },
  play: async ({ canvasElement, args }) => {
    const canvas = within(canvasElement);
    await userEvent.click(canvas.getByTestId("model-routing-reset-o4-mini"));
    await expect(args.onReset).toHaveBeenCalledWith("o4-mini");
  },
};

export const ReadOnly: Story = {
  args: { states, writesEnabled: false },
};

export const ErrorState: Story = {
  args: {
    states,
    error: "模型路由状态刷新失败，请稍后重试。",
  },
};
