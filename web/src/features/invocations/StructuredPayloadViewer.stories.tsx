import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import { StructuredPayloadViewer } from "./StructuredPayloadViewer";

const labels = {
  json: "JSON",
  ndjson: "NDJSON",
  sse: "SSE event stream",
  text: "Plain text",
  largePayload: "This payload is larger than 1 MiB. Raw text is shown to protect the interface.",
  parseLargePayload: "Parse structured content",
  event: "Event",
  data: "Data",
  expand: "Expand JSON",
  collapse: "Collapse JSON",
};

const meta = {
  title: "Components/StructuredPayloadViewer",
  component: StructuredPayloadViewer,
  tags: ["autodocs"],
  parameters: {
    layout: "padded",
    docs: {
      description: {
        component:
          "Read-only response-body inspector for JSON, NDJSON, SSE transcripts, and wrapping plain text.",
      },
    },
  },
  decorators: [
    (Story) => (
      <div className="mx-auto w-full max-w-4xl rounded-xl bg-base-200 p-4 text-base-content">
        <Story />
      </div>
    ),
  ],
  args: {
    value: JSON.stringify({ status: "failed", retryable: false, details: { code: 502 } }),
    labels,
  },
} satisfies Meta<typeof StructuredPayloadViewer>;

export default meta;
type Story = StoryObj<typeof meta>;

export const Json: Story = {};

export const Ndjson: Story = {
  args: {
    value: '{"event":"start","index":0}\n{"event":"done","index":1,"usage":{"tokens":42}}',
  },
};

export const SseTranscript: Story = {
  args: {
    value:
      'event: response.output_item.done\ndata: {"type":"response.output_item.done","output_index":0,"item":{"id":"msg_01","content":[{"type":"output_text","text":"A deliberately long response field remains inside the inspector rather than widening the drawer."}]}}\n\nevent: response.completed\ndata: {"type":"response.completed","response":{"status":"completed"}}',
  },
};

export const PlainTextWrap: Story = {
  args: {
    value: `upstream stream error: ${"unbroken-token-".repeat(24)}`,
  },
};

export const LargePayloadManualParse: Story = {
  args: {
    value: JSON.stringify({ payload: "x".repeat(1024 * 1024) }),
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const button = await canvas.findByRole("button", { name: labels.parseLargePayload });
    expect(canvas.queryByTestId("structured-payload-viewer")).toBeNull();
    await userEvent.click(button);
    expect(await canvas.findByTestId("structured-payload-viewer")).toHaveAttribute(
      "data-payload-kind",
      "json",
    );
  },
};
