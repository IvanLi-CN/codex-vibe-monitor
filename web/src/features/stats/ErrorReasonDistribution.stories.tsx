import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ErrorDistributionItem, FailureScope } from "../../lib/api";
import { ThemeProvider } from "../../theme";
import { ErrorReasonDistribution } from "./ErrorReasonDistribution";

const serviceItems: ErrorDistributionItem[] = [
  { reason: "Upstream request timed out before response headers were received", count: 212 },
  {
    reason: "The configured model is unavailable in every healthy upstream account pool",
    count: 148,
  },
  {
    reason: "Upstream closed the connection before the assistant response was complete",
    count: 102,
  },
  {
    reason: "No healthy account remained after retry and cooldown checks were exhausted",
    count: 76,
  },
  {
    reason: "The provider returned an invalid response while the stream was being decoded",
    count: 64,
  },
  {
    reason: "A temporary provider capacity limit prevented this request from being accepted",
    count: 52,
  },
  { reason: "The upstream response body ended before a complete message was received", count: 42 },
  {
    reason: "The configured route could not resolve this model to an available provider",
    count: 46,
  },
];

const clientItems: ErrorDistributionItem[] = [
  { reason: "The request did not include a model supported by the selected endpoint", count: 58 },
  { reason: "The supplied bearer token was rejected by the local authentication layer", count: 43 },
  { reason: "The request body could not be parsed as a supported JSON document", count: 32 },
  { reason: "The requested context length exceeds the configured request limit", count: 24 },
];

function InteractiveDistribution({ initialScope = "service" }: { initialScope?: FailureScope }) {
  const [scope, setScope] = useState<FailureScope>(initialScope);
  return (
    <ErrorReasonDistribution
      items={scope === "client" ? clientItems : serviceItems}
      isLoading={false}
      error={null}
      scope={scope}
      onScopeChange={setScope}
    />
  );
}

function StateGallery() {
  return (
    <div
      className="min-h-screen bg-base-200 p-8 text-base-content"
      data-visual-evidence-surface="error-reason-distribution-gallery"
    >
      <div
        className="mx-auto max-w-6xl space-y-8"
        data-visual-evidence-target="error-reason-distribution-gallery-target"
      >
        <section className="space-y-3">
          <h2 className="section-title">Default / Scope selection</h2>
          <InteractiveDistribution />
        </section>
        <div className="grid min-w-0 gap-8 lg:grid-cols-3">
          <section className="min-w-0 space-y-3">
            <h2 className="section-title">Loading</h2>
            <ErrorReasonDistribution
              items={serviceItems}
              isLoading
              error={null}
              scope="service"
              onScopeChange={() => undefined}
            />
          </section>
          <section className="min-w-0 space-y-3">
            <h2 className="section-title">Empty</h2>
            <ErrorReasonDistribution
              items={[]}
              isLoading={false}
              error={null}
              scope="service"
              onScopeChange={() => undefined}
            />
          </section>
          <section className="min-w-0 space-y-3">
            <h2 className="section-title">Error</h2>
            <ErrorReasonDistribution
              items={[]}
              isLoading={false}
              error="Error reasons could not be loaded. Try changing the failure scope."
              scope="service"
              onScopeChange={() => undefined}
            />
          </section>
        </div>
      </div>
    </div>
  );
}

const meta = {
  title: "Features/Stats/ErrorReasonDistribution",
  component: ErrorReasonDistribution,
  tags: ["autodocs"],
  args: {
    items: serviceItems,
    isLoading: false,
    error: null,
    scope: "service",
    onScopeChange: () => undefined,
  },
  decorators: [
    (Story) => (
      <I18nProvider initialLocale="zh" persistLocale={false}>
        <ThemeProvider>
          <div className="min-h-screen bg-base-200 p-8 text-base-content">
            <div className="mx-auto max-w-5xl">
              <Story />
            </div>
          </div>
        </ThemeProvider>
      </I18nProvider>
    ),
  ],
  parameters: {
    docs: {
      description: {
        component:
          "Error distribution with a scope selector, a pie chart, and a complete list of the returned reasons.",
      },
    },
  },
} satisfies Meta<typeof ErrorReasonDistribution>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {
  render: () => <InteractiveDistribution />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getAllByTestId("error-reason-row")).toHaveLength(8);
    await userEvent.click(canvas.getByTestId("stats-error-scope-select-trigger"));
    await userEvent.click(within(document.body).getByRole("option", { name: "调用方错误" }));
    await expect(
      canvas.getByText("The request did not include a model supported by the selected endpoint"),
    ).toBeVisible();
    await expect(canvas.getAllByTestId("error-reason-row")).toHaveLength(4);
  },
};

export const Loading: Story = {
  render: () => (
    <ErrorReasonDistribution
      items={serviceItems}
      isLoading
      error={null}
      scope="service"
      onScopeChange={() => undefined}
    />
  ),
};

export const Empty: Story = {
  render: () => (
    <ErrorReasonDistribution
      items={[]}
      isLoading={false}
      error={null}
      scope="service"
      onScopeChange={() => undefined}
    />
  ),
};

export const ErrorState: Story = {
  name: "Error",
  render: () => (
    <ErrorReasonDistribution
      items={[]}
      isLoading={false}
      error="Error reasons could not be loaded. Try changing the failure scope."
      scope="service"
      onScopeChange={() => undefined}
    />
  ),
};

export const DocsStateGallery: Story = {
  name: "State gallery",
  render: () => <StateGallery />,
};
