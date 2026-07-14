import type { Meta, StoryObj } from "@storybook/react-vite";
import { MemoryRouter } from "react-router-dom";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import { UPSTREAM_ACCOUNTS_CHANGED_EVENT } from "../../lib/upstreamAccountsEvents";
import UpstreamAccountsPage, {
  SharedUpstreamAccountDetailDrawer,
} from "../../pages/account-pool/UpstreamAccounts";
import { duplicateReasons } from "./UpstreamAccountsPage.story-data";
import {
  AccountPoolStoryRouter,
  StorybookUpstreamAccountsMock,
} from "./UpstreamAccountsPage.story-helpers";

const meta = {
  title: "Account Pool/Pages/Upstream Accounts/Overlays",
  component: UpstreamAccountsPage,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <SystemNotificationProvider>
          <StorybookUpstreamAccountsMock>
            <Story />
          </StorybookUpstreamAccountsMock>
        </SystemNotificationProvider>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof UpstreamAccountsPage>;

export default meta;

type Story = StoryObj<typeof meta>;

function detailRouteEntry(accountId: number, state?: Record<string, unknown>) {
  return {
    pathname: "/account-pool/upstream-accounts",
    search: `?upstreamAccountId=${accountId}`,
    state,
  };
}

async function findTokyoDetailDialog(documentScope: ReturnType<typeof within>) {
  const existingDialog = documentScope.queryByRole("dialog", {
    name: /Codex Pro - Tokyo/i,
  });
  if (existingDialog) return existingDialog;

  const routedDialog = await documentScope
    .findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    })
    .catch(() => null);
  if (routedDialog) return routedDialog;

  await userEvent.click(
    await documentScope.findByRole("button", {
      name: /选择 Codex Pro - Tokyo/i,
    }),
  );

  return await documentScope.findByRole("dialog", {
    name: /Codex Pro - Tokyo/i,
  });
}

async function expectFixedDesktopDrawerWidth(dialog: HTMLElement) {
  await expect(dialog).toHaveClass("drawer-shell--detail-wide");
  const width = dialog.getBoundingClientRect().width;
  await expect(width).toBeGreaterThan(0);
  return width;
}

function setTextboxValue(element: HTMLInputElement, value: string) {
  const view = element.ownerDocument.defaultView;
  const setter = view
    ? Object.getOwnPropertyDescriptor(view.HTMLInputElement.prototype, "value")?.set
    : undefined;

  setter?.call(element, value);
  element.dispatchEvent(new Event("input", { bubbles: true }));
  element.dispatchEvent(new Event("change", { bubbles: true }));
}

export const DetailDrawer: Story = {
  tags: ["test"],
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  parameters: {
    viewport: {
      defaultViewport: "desktop1920",
    },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    window.__storybookUpstreamAccountsController__?.clearRequestLog();
    const dialog = await findTokyoDetailDialog(documentScope);
    await expectFixedDesktopDrawerWidth(dialog);
    const initialRequestLog = window.__storybookUpstreamAccountsController__?.getRequestLog() ?? [];
    await expect(initialRequestLog.some((entry) => entry.includes("/sticky-keys"))).toBe(false);
    await expect(
      initialRequestLog.filter((entry) => entry.startsWith("GET /api/pool/upstream-accounts?"))
        .length,
    ).toBe(0);
    await expect(within(dialog).getByRole("tab", { name: /概览|overview/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await userEvent.click(within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }));
    await expectFixedDesktopDrawerWidth(dialog);
    await userEvent.click(within(dialog).getByRole("tab", { name: /路由|routing/i }));
    await expectFixedDesktopDrawerWidth(dialog);
    await waitFor(() => {
      const requestLog = window.__storybookUpstreamAccountsController__?.getRequestLog() ?? [];
      expect(requestLog.some((entry) => entry.includes("/sticky-keys"))).toBe(true);
    });
    await userEvent.click(within(dialog).getByRole("tab", { name: /健康与事件|health & events/i }));
    await expectFixedDesktopDrawerWidth(dialog);
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    await expectFixedDesktopDrawerWidth(dialog);
    await waitFor(() => {
      expect(dialog.querySelector('input[name="detailDisplayName"]')).not.toBeNull();
    });
  },
};

export const DetailDrawerOverview: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await expect(within(dialog).getByRole("tab", { name: /概览|overview/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(within(dialog).getByText(/图片能力|image capability/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/账号活动总览|account activity overview/i),
    ).toBeInTheDocument();
    await expect(
      within(dialog).getByTestId("upstream-account-records-activity-overview"),
    ).toBeInTheDocument();
  },
};

export const DetailDrawerRecordsPopulated: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }));
    await expect(
      within(dialog).queryByText(/账号活动总览|account activity overview/i),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByTestId("upstream-account-records-activity-overview"),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByRole("combobox", { name: /记录数量|rows/i }),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/pool upstream responded with 500/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/响应.*gpt-5\.4-2026-07-01|response.*gpt-5\.4-2026-07-01/i),
    ).toBeInTheDocument();
    await expect(
      within(dialog).queryByRole("columnheader", { name: /端点|endpoint/i }),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/连接 186 ms|Connect 186 ms/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByTestId("upstream-account-call-records-table"),
    ).toBeInTheDocument();
    await expect(
      within(dialog).getByRole("columnheader", { name: /错误|error/i }),
    ).toBeInTheDocument();
    await expect(within(dialog).getByText(/上游 HTTP 500|upstream http 500/i)).toBeInTheDocument();
    await expect(within(dialog).getByText("JP Edge 01")).toBeInTheDocument();
    await expect(within(dialog).getByText(/upstream_response_failed/i)).toBeInTheDocument();
    const desktopAttempts = within(dialog).getByTestId("upstream-account-call-records-table");
    await userEvent.click(
      within(within(desktopAttempts).getByTestId("account-attempt-evidence-9001")).getByText(
        /诊断详情|diagnostics/i,
      ),
    );
    await expect(within(dialog).getByText("upstream-story-500")).toBeInTheDocument();
    await expect(within(dialog).getByText("route-tokyo-primary")).toBeInTheDocument();
    await expect(within(dialog).getByText("502")).toBeInTheDocument();
  },
};

export const DetailDrawerEventLocatesAttempt: Story = {
  tags: ["test"],
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /健康与事件|health & events/i }));
    await userEvent.click(
      within(dialog).getByRole("button", {
        name: /上游尝试 ID.*9001|upstream attempt id.*9001/i,
      }),
    );
    const recordsTable = await within(dialog).findByTestId("upstream-account-call-records-table");
    const disclosure = within(recordsTable).getByTestId("account-attempt-evidence-9001");
    await expect(disclosure).toHaveAttribute("open");
    await expect(within(disclosure).getByText("upstream-story-500")).toBeInTheDocument();
  },
};

export const DetailDrawerHealthEventAttemptLink: Story = {
  render: () => <DetailDrawerStorySurface initialTab="healthEvents" />,
};

export const DetailDrawerRecordsMobile: Story = {
  ...DetailDrawerRecordsPopulated,
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }));
    await expect(
      within(dialog).getByTestId("upstream-account-call-records-mobile-table"),
    ).toBeInTheDocument();
    await expect(within(dialog).getByText(/上游 HTTP 500|upstream http 500/i)).toBeInTheDocument();
    const mobileAttempts = within(dialog).getByTestId("upstream-account-call-records-mobile-table");
    await userEvent.click(
      within(within(mobileAttempts).getByTestId("account-attempt-evidence-9001")).getByText(
        /诊断详情|diagnostics/i,
      ),
    );
    await expect(within(dialog).getByText("JP Edge 01")).toBeInTheDocument();
    await expect(within(dialog).getByText(/连接 186 ms|connect 186 ms/i)).toBeInTheDocument();
  },
};

export const DetailDrawerRecordsPendingMobile: Story = {
  ...DetailDrawerRecordsMobile,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }));
    await expect(within(dialog).getByText(/正在处理中|pending/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/等待首字节|waiting for first byte/i),
    ).toBeInTheDocument();
  },
};

export const DetailDrawerRecordsEmpty: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }));
    await expect(
      within(dialog).queryByText(/账号活动总览|account activity overview/i),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).getByText(/这个上游账号暂时还没有保留的调用记录|No retained call records/i),
    ).toBeInTheDocument();
  },
};

function DetailDrawerStorySurface({
  initialTab,
  initialDeleteConfirmOpen = false,
  maxWidth = "none",
  presentation = "overlay",
}: {
  initialTab: "overview" | "records" | "routing" | "healthEvents";
  initialDeleteConfirmOpen?: boolean;
  maxWidth?: string;
  presentation?: "overlay" | "page";
}) {
  const isPagePresentation = presentation === "page";

  return (
    <MemoryRouter initialEntries={["/account-pool/upstream-accounts?upstreamAccountId=101"]}>
      <div
        className={
          isPagePresentation
            ? "min-h-screen bg-base-100 text-base-content"
            : "min-h-screen bg-base-200 p-3 text-base-content min-[769px]:p-6"
        }
      >
        <I18nProvider>
          <SystemNotificationProvider>
            <StorybookUpstreamAccountsMock>
              <div
                style={!isPagePresentation && maxWidth !== "none" ? { maxWidth } : undefined}
                className={isPagePresentation ? "w-full" : "mx-auto w-full"}
              >
                <SharedUpstreamAccountDetailDrawer
                  open
                  accountId={101}
                  initialTab={initialTab}
                  initialDeleteConfirmOpen={initialDeleteConfirmOpen}
                  presentation={presentation}
                  onClose={() => {}}
                />
              </div>
            </StorybookUpstreamAccountsMock>
          </SystemNotificationProvider>
        </I18nProvider>
      </div>
    </MemoryRouter>
  );
}

export const DetailDrawerRecordsLoading: Story = {
  tags: ["test"],
  render: () => <DetailDrawerStorySurface initialTab="records" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expectFixedDesktopDrawerWidth(dialog);
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
  },
};

export const DetailDrawerRecordsSettled: Story = {
  tags: ["test"],
  render: () => <DetailDrawerStorySurface initialTab="records" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expectFixedDesktopDrawerWidth(dialog);
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
  },
};

export const DetailDrawerInvocationLocate: Story = {
  render: () => <DetailDrawerStorySurface initialTab="healthEvents" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    const invokeButton = await within(dialog).findByRole("button", {
      name: /上游尝试 ID.*9001|upstream attempt id.*9001/i,
    });
    await userEvent.click(invokeButton);
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
    await waitFor(() => {
      expect(
        dialog.querySelector('[data-invoke-id="inv_story_pool_failover_001"]'),
      ).toBeInTheDocument();
    });
    await expect(
      within(dialog).getByRole("button", {
        name: /返回最新记录|return to latest records/i,
      }),
    ).toBeInTheDocument();
  },
};

export const DetailDrawerInvocationLocateMobile: Story = {
  ...DetailDrawerInvocationLocate,
  parameters: {
    viewport: { defaultViewport: "mobile390" },
  },
};

export const DetailDrawerInvocationLocateReturnLatest: Story = {
  render: () => <DetailDrawerStorySurface initialTab="healthEvents" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await userEvent.click(
      await within(dialog).findByRole("button", {
        name: /上游尝试 ID.*9001|upstream attempt id.*9001/i,
      }),
    );
    await userEvent.click(
      await within(dialog).findByRole("button", {
        name: /返回最新记录|return to latest records/i,
      }),
    );
    await waitFor(() => {
      expect(
        within(dialog).queryByRole("button", {
          name: /返回最新记录|return to latest records/i,
        }),
      ).not.toBeInTheDocument();
    });
  },
};

export const DetailDrawerInvocationLocateNotFound: Story = {
  render: () => <DetailDrawerStorySurface initialTab="healthEvents" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await userEvent.click(
      await within(dialog).findByRole("button", {
        name: "inv_story_pool_failover_001",
      }),
    );
    const alert = await within(dialog).findByRole("alert");
    await expect(alert).toHaveTextContent("inv_story_pool_failover_001");
    await expect(alert).toHaveFocus();
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
  },
};

export const DetailDrawerRoutingRules: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /路由|routing/i }));
    await expect(within(dialog).getByRole("tab", { name: /路由|routing/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(
      within(dialog).queryByRole("button", { name: /编辑账号策略|edit account policy/i }),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByRole("button", { name: /账号路由策略|account routing policy/i }),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).getByText(/最终生效规则|effective routing rule/i),
    ).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/字段来源明细|field source breakdown/i),
    ).toBeInTheDocument();

    const warningValues = Array.from(
      dialog.querySelectorAll('[class*="bg-warning"]') as NodeListOf<HTMLElement>,
    ).map((node) => node.textContent);
    expect(warningValues).toContain("禁止切出");
    expect(warningValues).toContain("禁止切入");
    expect((dialog.textContent ?? "").match(/禁止切出/g)).toHaveLength(1);
    expect((dialog.textContent ?? "").match(/禁止切入/g)).toHaveLength(1);

    await userEvent.click(within(dialog).getByRole("button", { name: /添加代理|add proxy/i }));
    const proxyDialog = await documentScope.findByRole("dialog", {
      name: /选择账号代理节点|select account proxy nodes/i,
    });
    await expect(within(proxyDialog).getByText(/Direct/i)).toBeInTheDocument();
    await expect(within(proxyDialog).getByText(/fpn_5a7b0c1d2e3f4a10/i)).toBeInTheDocument();
    expect(within(proxyDialog).getAllByText(/24H/i).length).toBeGreaterThan(0);
    await expect(
      within(proxyDialog).getByRole("button", { name: /应用选择|apply selection/i }),
    ).toBeInTheDocument();
  },
};

export const DetailDrawerRecordsSettledWide: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1920" },
  },
  render: () => <DetailDrawerStorySurface initialTab="records" maxWidth="1920px" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(
      within(dialog).queryByText(/账号活动总览|account activity overview/i),
    ).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByTestId("upstream-account-records-activity-overview"),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/gpt-5\.4/i)).toBeInTheDocument();
  },
};

export const DetailPageMobile: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile430" },
  },
  render: () => <DetailDrawerStorySurface initialTab="routing" presentation="page" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/最终生效规则|effective routing rule/i)).toBeInTheDocument();
    await expect(within(document.body).queryByRole("dialog")).toBeNull();
  },
};

export const DetailPageMobileDeleteConfirm: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile430" },
  },
  render: () => (
    <DetailDrawerStorySurface initialTab="overview" initialDeleteConfirmOpen presentation="page" />
  ),
  play: async () => {
    const alertDialog = await within(document.body).findByRole("alertdialog");
    await expect(alertDialog).toHaveClass("dialog-surface");
    await expect(alertDialog).toHaveTextContent(/Codex Pro - Tokyo/i);
  },
};

export const DetailPageTablet: Story = {
  parameters: {
    viewport: { defaultViewport: "tablet768" },
  },
  render: () => <DetailDrawerStorySurface initialTab="overview" presentation="page" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/账号活动总览|account activity overview/i)).toBeInTheDocument();
    await expect(within(document.body).queryByRole("dialog")).toBeNull();
  },
};

export const DetailDrawerRecordsOverflowDarkNarrow: Story = {
  globals: {
    themeMode: "dark",
  },
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <DetailDrawerStorySurface initialTab="records" maxWidth="1280px" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(
      within(dialog).queryByText(/账号活动总览|account activity overview/i),
    ).not.toBeInTheDocument();
    await expect(within(dialog).queryByText("今日 Token")).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByRole("combobox", { name: /记录数量|rows/i }),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/gpt-5\.4/i)).toBeInTheDocument();
    await expect(within(dialog).queryByText(/并行对话|parallel/i)).not.toBeInTheDocument();
  },
};

export const DetailDrawerRecordsLoadingDarkNarrow: Story = {
  globals: {
    themeMode: "dark",
  },
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <DetailDrawerStorySurface initialTab="records" maxWidth="1280px" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
    await expect(
      within(dialog).queryByText(/账号活动总览|account activity overview/i),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByTestId("invocation-table-loading")).toBeInTheDocument();
    await expect(within(dialog).queryByText(/并行对话|parallel/i)).not.toBeInTheDocument();
  },
};

export const DetailDrawerRecordsInfinite: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
  render: () => <DetailDrawerStorySurface initialTab="records" maxWidth="1280px" />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Codex Pro - Tokyo/i,
    });
    await expect(
      within(dialog).getByRole("tab", { name: /上游调用|upstream calls/i }),
    ).toHaveAttribute("aria-selected", "true");
    await waitFor(() => {
      expect(within(dialog).getByText(/已加载 50 \/|Loaded 50 \//i)).toBeInTheDocument();
    });
    const body = dialog.querySelector(".drawer-body");
    if (!(body instanceof HTMLElement)) {
      throw new Error("missing drawer body");
    }
    body.scrollTop = body.scrollHeight;
    body.dispatchEvent(new Event("scroll", { bubbles: true }));
    await waitFor(() => {
      const requestLog = window.__storybookUpstreamAccountsController__?.getRequestLog() ?? [];
      expect(
        requestLog.some(
          (entry) =>
            entry.includes("/api/invocations") &&
            entry.includes("page=2") &&
            entry.includes("pageSize=50"),
        ),
      ).toBe(true);
    });
  },
};

export const EditDraftSurvivesBackgroundRefresh: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    await waitFor(() => {
      expect(dialog.querySelector('input[name="detailDisplayName"]')).not.toBeNull();
    });
    const displayNameInput = dialog.querySelector(
      'input[name="detailDisplayName"]',
    ) as HTMLInputElement;
    setTextboxValue(displayNameInput, "Codex Pro - Tokyo Draft");

    window.dispatchEvent(new CustomEvent(UPSTREAM_ACCOUNTS_CHANGED_EVENT));

    await waitFor(() => {
      const refreshedInput = dialog.querySelector(
        'input[name="detailDisplayName"]',
      ) as HTMLInputElement;
      expect(refreshedInput.value).toBe("Codex Pro - Tokyo Draft");
    });
  },
};

export const AccountPolicyDraftSurvivesBackgroundRefresh: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /路由|routing/i }));
    await expect(
      within(dialog).queryByRole("button", { name: /编辑账号策略|edit account policy/i }),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/账号代理|account forward proxies/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/账号覆盖|account override/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/DIRECT/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/fpn_5a7b0c1d2e3f4a10/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/连续网络失败|consecutive network failures/i),
    ).toBeInTheDocument();

    window.dispatchEvent(new CustomEvent(UPSTREAM_ACCOUNTS_CHANGED_EVENT));

    await waitFor(() => {
      expect(
        within(dialog).queryByRole("button", { name: /编辑账号策略|edit account policy/i }),
      ).not.toBeInTheDocument();
      expect(within(dialog).getByText(/账号代理|account forward proxies/i)).toBeInTheDocument();
      expect(within(dialog).getByText(/DIRECT/i)).toBeInTheDocument();
    });
  },
};

export const OauthEmailOverview: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts",
        search: "?upstreamAccountId=101",
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    await expect(within(dialog).getByDisplayValue("old@storybook.example.com")).toBeInTheDocument();
    await expect(within(dialog).getByText(/verified@storybook\.example\.com/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/latest oauth verification reported|最近一次 oauth 可信邮箱是/i),
    ).toBeInTheDocument();
  },
};

export const DetailDrawerReadOnlySystemTags: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await findTokyoDetailDialog(documentScope);
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    await expect(
      within(dialog).queryByRole("button", {
        name: /添加 tag|add tag/i,
      }),
    ).not.toBeInTheDocument();
    await expect(within(dialog).getByText(/不支持 gpt-5\.5/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/不支持 WS/i)).toBeInTheDocument();

    window.dispatchEvent(new CustomEvent(UPSTREAM_ACCOUNTS_CHANGED_EVENT));

    await waitFor(() => {
      expect(within(dialog).getByText(/不支持 gpt-5\.5/i)).toBeInTheDocument();
      expect(within(dialog).getByText(/不支持 WS/i)).toBeInTheDocument();
    });
  },
};

export const DetailDrawerStickyHistory: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    await userEvent.click(within(dialog).getByRole("tab", { name: /路由|routing/i }));
    await userEvent.click(
      within(dialog).getAllByRole("button", {
        name: /打开全部调用记录|open full call history/i,
      })[0],
    );
    await expect(
      documentScope.getByText(/019ce3a1-6787-7910-b0fd-c246d6f6a901/i),
    ).toBeInTheDocument();
    await expect(documentScope.getByText(/gpt-5\.4/i)).toBeInTheDocument();
  },
};

export const MissingWindowPlaceholders: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(102)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", {
      name: /Team key - missing weekly limit/i,
    });
    await expect(
      within(dialog).queryByText(/图片工具能力|image tool capability/i),
    ).not.toBeInTheDocument();
    await expect(within(dialog).queryByText(/账号 ID|Account ID/i)).not.toBeInTheDocument();
    await expect(within(dialog).queryByText(/User ID/i)).not.toBeInTheDocument();
    await expect(within(dialog).queryByText(/5 小时窗口|5h window/i)).not.toBeInTheDocument();
    await expect(within(dialog).queryByText(/7 天窗口|7d window/i)).not.toBeInTheDocument();
    await expect(within(dialog).queryByText(/18 requests/i)).not.toBeInTheDocument();
    await expect(
      within(dialog).queryByText(/还没有额度历史|No quota history yet/i),
    ).not.toBeInTheDocument();
  },
};

export const DeleteConfirmation: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    await expect(documentScope.getByRole("alertdialog")).toBeInTheDocument();
    await expect(
      documentScope.getByText(/确认删除 Codex Pro - Tokyo|delete Codex Pro - Tokyo/i),
    ).toBeInTheDocument();
  },
};

export const DeleteFailure: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    const confirmDialog = await documentScope.findByRole("alertdialog");
    await userEvent.click(
      within(confirmDialog).getByRole("button", { name: /确认删除|delete account/i }),
    );
    await expect(within(dialog).getByText(/database is locked/i)).toBeInTheDocument();
    await expect(documentScope.queryByRole("alertdialog")).not.toBeInTheDocument();
  },
};

export const DeleteSuccessClosesDrawer: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={detailRouteEntry(101, {
        selectedAccountId: 101,
        openDeleteConfirm: true,
      })}
    />
  ),
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    const confirmDialog = await documentScope.findByRole("alertdialog");
    await userEvent.click(
      within(confirmDialog).getByRole("button", { name: /确认删除|delete account/i }),
    );
    await waitFor(() => {
      expect(documentScope.queryByRole("dialog", { name: /Codex Pro - Tokyo/i })).toBeNull();
    });
    await expect(documentScope.queryByRole("alertdialog")).toBeNull();
    await expect(
      documentScope.getByRole("heading", { name: /upstream accounts|上游账号/i }),
    ).toBeInTheDocument();
  },
};

export const RoutingDialog: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);
    const editButton = await canvas.findByRole("button", {
      name: /编辑路由设置|edit routing settings/i,
    });
    await userEvent.click(editButton);
    const dialog = documentScope.getByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    await expect(dialog).toBeInTheDocument();
    const generateButton = within(dialog).getByRole("button", { name: /生成密钥|generate key/i });
    await expect(generateButton).toBeInTheDocument();
    await expect(
      within(dialog).getByLabelText(/优先队列同步间隔|priority sync interval/i),
    ).toBeInTheDocument();
    await expect(
      within(dialog).getByLabelText(/次级队列同步间隔|secondary sync interval/i),
    ).toBeInTheDocument();
    await expect(
      within(dialog).getByLabelText(/优先可用账号上限|priority available account cap/i),
    ).toBeInTheDocument();
    await userEvent.click(generateButton);
    const input = within(dialog).getByPlaceholderText(
      /粘贴新的号池 API Key|paste a new pool api key/i,
    ) as HTMLInputElement;
    await expect(input.value).toMatch(/^cvm-[0-9a-f]{32}$/);
    await userEvent.click(within(dialog).getByRole("button", { name: /取消|cancel/i }));
    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const reopenedDialog = documentScope.getByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    const reopenedInput = within(reopenedDialog).getByPlaceholderText(
      /粘贴新的号池 API Key|paste a new pool api key/i,
    ) as HTMLInputElement;
    await expect(reopenedInput.value).toBe("");
  },
};

export const RoutingDialogMaintenanceOnlySave: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const dialog = await documentScope.findByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    const primaryInput = within(dialog).getByLabelText(
      /优先队列同步间隔|priority sync interval/i,
    ) as HTMLInputElement;
    const secondaryInput = within(dialog).getByLabelText(
      /次级队列同步间隔|secondary sync interval/i,
    ) as HTMLInputElement;
    const capInput = within(dialog).getByLabelText(
      /优先可用账号上限|priority available account cap/i,
    ) as HTMLInputElement;
    const apiKeyInput = within(dialog).getByPlaceholderText(
      /粘贴新的号池 API Key|paste a new pool api key/i,
    ) as HTMLInputElement;
    const saveButton = within(dialog).getByRole("button", { name: /保存设置|save settings/i });

    await expect(primaryInput.value).toBe("300");
    await expect(secondaryInput.value).toBe("1800");
    await expect(capInput.value).toBe("100");
    await expect(apiKeyInput.value).toBe("");
    await expect(saveButton).toBeDisabled();

    await userEvent.clear(primaryInput);
    await userEvent.type(primaryInput, "600");
    await userEvent.clear(secondaryInput);
    await userEvent.type(secondaryInput, "2400");
    await userEvent.clear(capInput);
    await userEvent.type(capInput, "42");

    await expect(saveButton).toBeEnabled();
    await userEvent.click(saveButton);

    await waitFor(() => {
      expect(
        documentScope.queryByRole("dialog", {
          name: /高级路由与同步设置|advanced routing & sync settings/i,
        }),
      ).not.toBeInTheDocument();
    });

    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const reopenedDialog = await documentScope.findByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /优先队列同步间隔|priority sync interval/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("600");
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /次级队列同步间隔|secondary sync interval/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("2400");
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /优先可用账号上限|priority available account cap/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("42");
  },
};

export const RoutingDialogTimeoutSettings: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const dialog = await documentScope.findByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    const responsesFirstByteInput = within(dialog).getByLabelText(
      /一般请求响应体首字超时|standard response first byte timeout/i,
    ) as HTMLInputElement;
    const compactFirstByteInput = within(dialog).getByLabelText(
      /压缩请求响应体首字超时|compact response first byte timeout/i,
    ) as HTMLInputElement;
    const responsesStreamInput = within(dialog).getByLabelText(
      /一般请求流结束超时|standard stream completion timeout/i,
    ) as HTMLInputElement;
    const compactStreamInput = within(dialog).getByLabelText(
      /压缩请求流结束超时|compact stream completion timeout/i,
    ) as HTMLInputElement;
    const saveButton = within(dialog).getByRole("button", { name: /保存设置|save settings/i });

    await expect(responsesFirstByteInput.value).toBe("120");
    await expect(compactFirstByteInput.value).toBe("300");
    await expect(responsesStreamInput.value).toBe("300");
    await expect(compactStreamInput.value).toBe("300");

    await userEvent.clear(responsesFirstByteInput);
    await userEvent.type(responsesFirstByteInput, "180");
    await userEvent.clear(compactFirstByteInput);
    await userEvent.type(compactFirstByteInput, "420");
    await userEvent.clear(responsesStreamInput);
    await userEvent.type(responsesStreamInput, "360");
    await userEvent.clear(compactStreamInput);
    await userEvent.type(compactStreamInput, "540");

    await expect(saveButton).toBeEnabled();
    await userEvent.click(saveButton);

    await waitFor(() => {
      expect(
        documentScope.queryByRole("dialog", {
          name: /高级路由与同步设置|advanced routing & sync settings/i,
        }),
      ).not.toBeInTheDocument();
    });

    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const reopenedDialog = await documentScope.findByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /一般请求响应体首字超时|standard response first byte timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("180");
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /压缩请求响应体首字超时|compact response first byte timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("420");
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /一般请求流结束超时|standard stream completion timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("360");
    await expect(
      (
        within(reopenedDialog).getByLabelText(
          /压缩请求流结束超时|compact stream completion timeout/i,
        ) as HTMLInputElement
      ).value,
    ).toBe("540");
  },
};

export const RoutingDialogValidation: Story = {
  render: () => <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts" />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await userEvent.click(
      await canvas.findByRole("button", { name: /编辑路由设置|edit routing settings/i }),
    );
    const dialog = await documentScope.findByRole("dialog", {
      name: /高级路由与同步设置|advanced routing & sync settings/i,
    });
    const primaryInput = within(dialog).getByLabelText(
      /优先队列同步间隔|priority sync interval/i,
    ) as HTMLInputElement;
    const secondaryInput = within(dialog).getByLabelText(
      /次级队列同步间隔|secondary sync interval/i,
    ) as HTMLInputElement;
    const saveButton = within(dialog).getByRole("button", { name: /保存设置|save settings/i });

    await userEvent.clear(primaryInput);
    await userEvent.type(primaryInput, "600");
    await userEvent.clear(secondaryInput);
    await userEvent.type(secondaryInput, "300");

    await expect(
      within(dialog).getByText(
        /次级队列同步间隔必须大于等于优先队列同步间隔|secondary sync interval must be greater than or equal to the priority sync interval/i,
      ),
    ).toBeInTheDocument();
    await expect(saveButton).toBeDisabled();
  },
};

export const CompactSupportDetailDrawer: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await expect(
      await canvas.findByText(/Compact 不支持|Compact unsupported/i),
    ).toBeInTheDocument();
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    await userEvent.click(within(dialog).getByRole("tab", { name: /健康与事件|health & events/i }));
    await expect(within(dialog).getByText(/Compact 支持|Compact support/i)).toBeInTheDocument();
    await expect(within(dialog).getByText(/不支持|unsupported/i)).toBeInTheDocument();
    await expect(
      within(dialog).getByText(/No available channel for model gpt-5\.4-openai-compact/i),
    ).toBeInTheDocument();
  },
};
export const DetailDrawerGroupNotes: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = await documentScope.findByRole("dialog", { name: /Codex Pro - Tokyo/i });
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    await userEvent.click(
      await within(dialog).findByRole("button", {
        name: /编辑分组设置|edit group settings|编辑分组备注|edit group note/i,
      }),
    );
    await expect(
      documentScope.getByRole("dialog", { name: /分组设置|group settings|分组备注|group note/i }),
    ).toBeInTheDocument();
    await expect(documentScope.getByText(/production/i)).toBeInTheDocument();
  },
};

export const DetailDrawerApiKeyInvalidUpstreamUrl: Story = {
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(102)} />,
  play: async ({ canvasElement }) => {
    const documentScope = within(canvasElement.ownerDocument.body);
    const dialog = documentScope.getByRole("dialog", { name: /Team key - staging/i });
    await userEvent.click(within(dialog).getByRole("tab", { name: /编辑|edit/i }));
    const field = within(dialog).getByLabelText(/upstream base url/i);
    await userEvent.clear(field);
    await userEvent.type(field, "https://proxy.example.com/gateway?team=staging");
    await expect(
      documentScope.getByText(/cannot include a query string or fragment|不能包含查询串或片段/i),
    ).toBeInTheDocument();
    await expect(within(dialog).getByRole("button", { name: /save changes/i })).toBeDisabled();
  },
};

export const DuplicateOauthWarning: Story = {
  name: "Duplicate OAuth Warning",
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts",
        state: {
          selectedAccountId: 101,
          duplicateWarning: {
            accountId: 101,
            displayName: "Codex Pro - Tokyo",
            peerAccountIds: [103],
            reasons: [...duplicateReasons],
          },
        },
      }}
    />
  ),
};

export const DuplicateOauthDetail: Story = {
  name: "Duplicate OAuth Detail",
  render: () => <AccountPoolStoryRouter initialEntry={detailRouteEntry(101)} />,
};
