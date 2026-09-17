import type { Meta, StoryObj } from "@storybook/react-vite";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { SystemNotificationProvider } from "../../components/ui/system-notifications";
import { I18nProvider } from "../../i18n";
import AccountPoolLayout from "../../pages/account-pool/AccountPoolLayout";
import UpstreamAccountsPage from "../../pages/account-pool/UpstreamAccounts";
import { InvocationTable } from "./InvocationTable";
import {
  InvocationTableStoryShell,
  records,
  StorybookInvocationTableMock,
} from "./InvocationTable.stories.support-base";

const meta = {
  title: "Monitoring/InvocationCardList",
  component: InvocationTable,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
    docs: {
      description: {
        component:
          "Shows recent invocation records as compact three-segment cards with status, account attribution, proxy metadata, `TTFT / 响应耗时 / 请求压缩` summaries, and expandable request details. Conversation identity is intentionally supplied by the surrounding drawer and is not repeated in each card. The default story includes pool-routed and reverse-proxy records so you can verify model routing, reasoning/FAST signals, token/cost fields, and the account drawer trigger. The standalone Records search table is not this component.\n\nUse this component to verify desktop and 393px card wrapping, the running-to-terminal live update story, unavailable in-flight timings, and the expanded detail section for request metadata, timing stages, account attribution, request compression, and response diagnostics.",
      },
    },
  },
  argTypes: {
    records: {
      control: "object",
      description:
        "Invocation rows rendered by the card list. Include `reasoningEffort` and `reasoningTokens` to exercise the diagnostic segment; missing values render as `—`.",
      table: {
        type: { summary: "ApiInvocation[]" },
      },
    },
    isLoading: {
      control: "boolean",
      description: "Displays the loading spinner state while the card list is waiting for records.",
      table: {
        type: { summary: "boolean" },
        defaultValue: { summary: "false" },
      },
    },
    error: {
      control: "text",
      description:
        "Optional request error message rendered above the card list when loading fails.",
      table: {
        type: { summary: "string | null" },
        defaultValue: { summary: "null" },
      },
    },
  },
  decorators: [
    (Story) => (
      <I18nProvider>
        <MemoryRouter initialEntries={["/dashboard"]}>
          <SystemNotificationProvider>
            <StorybookInvocationTableMock>
              <Routes>
                <Route
                  path="/dashboard"
                  element={
                    <InvocationTableStoryShell>
                      <Story />
                    </InvocationTableStoryShell>
                  }
                />
                <Route path="/account-pool" element={<AccountPoolLayout />}>
                  <Route path="upstream-accounts" element={<UpstreamAccountsPage />} />
                </Route>
              </Routes>
            </StorybookInvocationTableMock>
          </SystemNotificationProvider>
        </MemoryRouter>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationTable>;

type Story = StoryObj<typeof meta>;

const defaultArgs: Story["args"] = {
  records,
  isLoading: false,
  error: null,
};

export type { Story };
export { defaultArgs, meta };
