/* eslint-disable @typescript-eslint/ban-ts-comment, @typescript-eslint/no-unused-vars */
// @ts-nocheck

import { act } from "react";
import { createRoot } from "react-dom/client";
import { MemoryRouter } from "react-router-dom";
import { expect, it, vi } from "vitest";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import { ThemeProvider } from "../../theme/context";
import { SharedUpstreamAccountDetailDrawer } from "./UpstreamAccounts";
import {
  flushAsync,
  mockAccountsPage,
  navigateMock,
  root,
  setHost,
  setRoot,
} from "./UpstreamAccounts.test-support";

it("labels a linked health event with its upstream attempt id", async () => {
  mockAccountsPage({
    detail: {
      recentActions: [
        {
          id: 71,
          occurredAt: "2026-03-16T02:06:00.000Z",
          action: "route_cooldown_started",
          source: "call",
          reasonCode: "upstream_http_5xx",
          reasonMessage: "Service temporarily unavailable",
          httpStatus: 503,
          failureKind: "upstream_http_5xx",
          invokeId: "invk_action_001",
          attemptId: "4V7MYPJG",
          model: "gpt-5.6-terra",
          createdAt: "2026-03-16T02:06:00.000Z",
        },
      ],
    },
  });

  const nextHost = document.createElement("div");
  setHost(nextHost);
  document.body.appendChild(nextHost);
  setRoot(createRoot(nextHost));
  act(() => {
    root?.render(
      <ThemeProvider>
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                initialTab="healthEvents"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>
      </ThemeProvider>,
    );
  });

  await flushAsync();
  expect(document.body.textContent).toMatch(/上游尝试 ID|Upstream attempt ID/);
  expect(document.body.textContent).toContain("4V7MYPJG");
  expect(document.body.textContent).not.toMatch(/请求 ID: invk_action_001/);
  expect(document.body.textContent).not.toMatch(/请求模型|Request model/);
  const impact = document.querySelector('[data-testid="account-event-impact"]');
  expect(impact?.textContent).toMatch(/影响范围.*账号|Impact scope.*Account/);
  expect(impact?.textContent).toMatch(/受影响模型.*全部|Affected models.*All/);
  expect(impact?.textContent).not.toMatch(/其他模型|Other models/);
  expect(impact?.querySelectorAll('[data-testid="account-event-impact-chip"]')).toHaveLength(2);
  expect(impact?.parentElement?.getAttribute("data-testid")).toBe("account-event-meta");
  expect(document.body.textContent).toMatch(/调用|Call/);
  expect(document.body.textContent).not.toMatch(/operator|maintenance_scheduler/);
  expect(document.body.textContent).toMatch(/上游服务异常|Upstream service failure/);
  expect(document.body.textContent).not.toContain("Service temporarily unavailable");
  expect(document.body.textContent).not.toContain("gpt-5.6-terra");
});
it("omits impact text for informational account events", async () => {
  mockAccountsPage({
    detail: {
      recentActions: [
        {
          id: 72,
          occurredAt: "2026-03-16T02:07:00.000Z",
          action: "account_updated",
          source: "account_update",
          result: "success",
          reasonCode: "account_updated",
          reasonMessage: "Settings saved",
          createdAt: "2026-03-16T02:07:00.000Z",
        },
      ],
    },
  });

  const nextHost = document.createElement("div");
  setHost(nextHost);
  document.body.appendChild(nextHost);
  setRoot(createRoot(nextHost));
  act(() => {
    root?.render(
      <ThemeProvider>
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                initialTab="healthEvents"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>
      </ThemeProvider>,
    );
  });

  await flushAsync();
  expect(document.body.textContent).toMatch(/账号设置已更新|Account settings were updated/);
  expect(document.body.textContent).not.toContain("Settings saved");
  expect(document.querySelector('[data-testid="account-event-impact"]')).toBeNull();
});
it("shows a model recovery transition without claiming an active impact", async () => {
  mockAccountsPage({
    detail: {
      recentActions: [
        {
          id: 73,
          occurredAt: "2026-03-16T02:08:00.000Z",
          action: "model_route_recovered",
          source: "call",
          result: "recovered",
          reasonCode: "sync_ok",
          model: "gpt-5.4-mini",
          modelRouteStateBefore: "cooling_down",
          modelRouteStateAfter: "available",
          modelRoutePriorityBefore: "excluded",
          modelRoutePriorityAfter: "normal",
          modelRouteFailureCount: 0,
          createdAt: "2026-03-16T02:08:00.000Z",
        },
      ],
    },
  });

  const nextHost = document.createElement("div");
  setHost(nextHost);
  document.body.appendChild(nextHost);
  setRoot(createRoot(nextHost));
  act(() => {
    root?.render(
      <ThemeProvider>
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                initialTab="healthEvents"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>
      </ThemeProvider>,
    );
  });

  await flushAsync();
  expect(document.body.textContent).toContain("gpt-5.4-mini");
  expect(document.body.textContent).toMatch(
    /冷却中.*可用.*排除.*正常|Cooling down.*Available.*Excluded.*Normal/,
  );
  expect(document.body.textContent).not.toMatch(/cooling_down|model_route/);
  expect(document.querySelector('[data-testid="account-event-impact"]')).toBeNull();
});
it("opens the blocked-binding working conversation filter from a health event", async () => {
  mockAccountsPage({
    detail: {
      recentActions: [
        {
          id: 71,
          occurredAt: "2026-03-16T02:06:00.000Z",
          action: "route_hard_unavailable",
          source: "call",
          reasonCode: "pool_assigned_account_blocked",
          reasonMessage: "This conversation is pinned to one blocked account",
          httpStatus: 502,
          failureKind: "pool_assigned_account_blocked",
          invokeId: "invk_action_001",
          attemptId: "4V7MYPJG",
          stickyKey: "sticky-dup-001",
          createdAt: "2026-03-16T02:06:00.000Z",
          blockedBinding: {
            upstreamAccountId: 2890,
            constraintSource: "encryptedSessionOwner",
          },
        },
      ],
    },
  });

  const nextHost = document.createElement("div");
  setHost(nextHost);
  document.body.appendChild(nextHost);
  setRoot(createRoot(nextHost));
  act(() => {
    root?.render(
      <ThemeProvider>
        <I18nProvider>
          <SystemNotificationProvider>
            <MemoryRouter>
              <SharedUpstreamAccountDetailDrawer
                open
                accountId={5}
                initialTab="healthEvents"
                onClose={vi.fn()}
              />
            </MemoryRouter>
          </SystemNotificationProvider>
        </I18nProvider>
      </ThemeProvider>,
    );
  });

  await flushAsync();
  expect(document.body.textContent).toMatch(/加密 owner 约束|Encrypted owner constraint/);

  const openButton = document.body.querySelector(
    '[data-testid="upstream-account-recent-action-open-blocked-binding"]',
  );
  if (!(openButton instanceof HTMLButtonElement)) {
    throw new Error("missing blocked binding action button");
  }

  act(() => {
    openButton.click();
  });

  expect(navigateMock).toHaveBeenCalledWith({
    pathname: "/dashboard",
    search:
      "?blockedBindingUpstreamAccountId=2890&blockedBindingConstraintSource=encryptedSessionOwner",
  });
});
