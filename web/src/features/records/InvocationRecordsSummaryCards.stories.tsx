import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import { InvocationRecordsSummaryCards } from "./InvocationRecordsSummaryCards";
import { createStoryInvocationRecordsSummary } from "./invocationRecordsStoryFixtures";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="app-shell-boundary">{children}</div>
    </div>
  );
}

const meta = {
  title: "Records/InvocationRecordsSummaryCards",
  component: InvocationRecordsSummaryCards,
  tags: ["autodocs"],
  decorators: [
    (Story) => (
      <I18nProvider>
        <StorySurface>
          <Story />
        </StorySurface>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof InvocationRecordsSummaryCards>;

export default meta;

type Story = StoryObj<typeof meta>;

export const TokenFocus: Story = {
  args: {
    focus: "token",
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
};

export const TtftAndResponseDuration: Story = {
  args: {
    focus: "network",
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
  tags: ["test"],
  parameters: {
    docs: {
      description: {
        story:
          "The records network summary uses independently aggregated TTFT and upstream response duration. TTFB and total time remain diagnostics, not primary metrics.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);

    await expect(canvas.getByText(/Avg TTFT|平均 TTFT/)).toBeVisible();
    await expect(canvas.getByText(/P95 TTFT|P95 TTFT/)).toBeVisible();
    await expect(canvas.getByText(/Avg response time|平均响应耗时/)).toBeVisible();
    await expect(canvas.getByText(/P95 response time|P95 响应耗时/)).toBeVisible();
    await expect(canvas.queryByText(/Avg TTFB|平均 TTFB/)).not.toBeInTheDocument();
    await expect(canvas.queryByText(/Avg total time|平均总耗时/)).not.toBeInTheDocument();
  },
};

export const ExceptionFocus: Story = {
  args: {
    focus: "exception",
    summary: createStoryInvocationRecordsSummary(),
    isLoading: false,
    error: null,
  },
};

export const Loading: Story = {
  args: {
    focus: "token",
    summary: null,
    isLoading: true,
    error: null,
  },
};

export const LoadError: Story = {
  args: {
    focus: "token",
    summary: null,
    isLoading: false,
    error: "Request failed: 500 database is busy",
  },
};
