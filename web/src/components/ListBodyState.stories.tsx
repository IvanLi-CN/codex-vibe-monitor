import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, within } from "storybook/test";
import { ListBodyState } from "./ListBodyState";

const meta = {
  title: "Components/ListBodyState",
  component: ListBodyState,
  parameters: {
    layout: "centered",
  },
  args: {
    title: "Loading records",
    description: "The list is preparing its first page.",
    testId: "storybook-list-body-state",
  },
  decorators: [
    (Story) => (
      <div className="w-[min(54rem,calc(100vw-2rem))]">
        <Story />
      </div>
    ),
  ],
} satisfies Meta<typeof ListBodyState>;

export default meta;

type Story = StoryObj<typeof meta>;

export const LoadingSkeleton: Story = {
  args: {
    variant: "loading",
    title: "Loading records",
    description: "The list is preparing its first page.",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("storybook-list-body-state")).toHaveAttribute("aria-busy", "true");
    await expect(canvas.getByText("Loading records")).toBeInTheDocument();
  },
};

export const InitialError: Story = {
  args: {
    variant: "error",
    title: "Failed to load records",
    description: "Request failed: 400 Failed to deserialize query string.",
    retryLabel: "Retry",
    onRetry: () => undefined,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByRole("alert")).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  },
};

export const EmptyResult: Story = {
  args: {
    variant: "empty",
    title: "No records yet",
    description: "Successful response, but no rows matched the current filters.",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("No records yet")).toBeInTheDocument();
  },
};
