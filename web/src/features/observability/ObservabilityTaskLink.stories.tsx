import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import { StorybookPageEnvironment } from "../../storybook/storybookPageHelpers";
import { ObservabilityTaskLink } from "./ObservabilityTaskLink";

const meta = {
  title: "Observability/TaskLink",
  component: ObservabilityTaskLink,
  tags: ["autodocs"],
  args: { taskKey: "retention_archive" },
  parameters: { layout: "fullscreen", grafanaConfigured: true },
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <StorybookPageEnvironment
          onRequest={({ url }) =>
            url.pathname === "/api/system/observability"
              ? Response.json({
                  enabled: true,
                  state: "enabled",
                  grafanaPublicUrl: context.parameters.grafanaConfigured
                    ? "https://grafana.example.invalid"
                    : null,
                  grafanaConnectivity: "unknown",
                  hotpath: true,
                  dashboards: ["cvm-runtime"],
                  datasourceUid: "cvm-prometheus",
                  variables: ["task_key"],
                })
              : undefined
          }
        >
          <div data-visual-evidence-surface className="w-full max-w-xl bg-base-200 p-6">
            <div data-visual-evidence-target>
              <Story />
            </div>
          </div>
        </StorybookPageEnvironment>
      </I18nProvider>
    ),
  ],
} satisfies Meta<typeof ObservabilityTaskLink>;
export default meta;
type Story = StoryObj<typeof meta>;

export const Configured: Story = {
  tags: ["test"],
  play: async ({ canvasElement }) => {
    const link = await within(canvasElement).findByRole("link");
    await expect(link).toHaveAttribute("target", "_blank");
    const url = new URL(link.getAttribute("href") ?? "");
    await expect(url.pathname).toBe("/d/cvm-runtime");
    await expect(url.searchParams.get("var-task_key")).toBe("retention_archive");
    await expect(url.searchParams.get("timezone")).toBe("utc");
  },
};

export const Unconfigured: Story = {
  tags: ["test"],
  parameters: { grafanaConfigured: false },
  play: async ({ canvasElement }) => {
    await expect(within(canvasElement).queryByRole("link")).not.toBeInTheDocument();
    await expect(
      within(canvasElement).getByTestId("task-observability-link"),
    ).not.toBeEmptyDOMElement();
  },
};

export const Narrow: Story = {
  ...Configured,
  parameters: { viewport: { defaultViewport: "mobile390" } },
};
