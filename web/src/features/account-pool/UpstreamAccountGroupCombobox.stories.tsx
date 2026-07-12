import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import type { UpstreamAccountGroupOption } from "../../lib/upstreamAccountGroups";
import { UpstreamAccountGroupCombobox } from "./UpstreamAccountGroupCombobox";

function ComboboxHarness({
  value: initialValue,
  suggestions,
  options,
  placeholder,
  createLabel,
  onCreateRequested,
  formatAccountCountLabel,
}: {
  value: string;
  suggestions: string[];
  options?: UpstreamAccountGroupOption[];
  placeholder?: string;
  createLabel?: (value: string) => string;
  onCreateRequested?: (value: string) => void;
  formatAccountCountLabel?: (count: number) => string;
}) {
  const [value, setValue] = useState(initialValue);
  const [lastCreateRequest, setLastCreateRequest] = useState<string | null>(null);
  return (
    <div className="min-h-screen bg-base-200 px-6 py-8 text-base-content">
      <div className="mx-auto max-w-md">
        <UpstreamAccountGroupCombobox
          value={value}
          suggestions={suggestions}
          options={options}
          placeholder={placeholder}
          createLabel={createLabel}
          onValueChange={setValue}
          onCreateRequested={(nextValue) => {
            setLastCreateRequest(nextValue);
            onCreateRequested?.(nextValue);
          }}
          formatAccountCountLabel={formatAccountCountLabel}
        />
        <p className="mt-3 text-sm text-base-content/70">
          Last create request: {lastCreateRequest ?? "none"}
        </p>
      </div>
    </div>
  );
}

const meta = {
  title: "Account Pool/Components/Upstream Account Group Combobox",
  component: UpstreamAccountGroupCombobox,
  tags: ["autodocs"],
  parameters: {
    layout: "fullscreen",
  },
  args: {
    value: "",
    onValueChange: () => undefined,
    suggestions: ["production", "staging", "shared-services"],
    options: [
      { groupName: "production", accountCount: 8, isPersisted: true },
      { groupName: "staging", accountCount: 2, isPersisted: true },
      { groupName: "shared-services", accountCount: 0, isPersisted: true },
    ],
    placeholder: "Select or type a group",
    createLabel: (value: string) => `Configure "${value}"`,
    formatAccountCountLabel: (count: number) => `${count} accounts`,
  },
  render: (args) => (
    <ComboboxHarness
      value={args.value}
      suggestions={args.suggestions ?? []}
      options={args.options}
      placeholder={args.placeholder}
      createLabel={args.createLabel}
      onCreateRequested={args.onCreateRequested}
      formatAccountCountLabel={args.formatAccountCountLabel}
    />
  ),
} satisfies Meta<typeof UpstreamAccountGroupCombobox>;

export default meta;

type Story = StoryObj<typeof meta>;

export const Default: Story = {};

export const WithExistingValue: Story = {
  args: {
    value: "production",
  },
};

export const CreateRequestFlow: Story = {
  args: {
    value: "production",
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await userEvent.click(canvas.getByRole("combobox"));
    const searchInput = documentScope.getByRole("textbox");
    await userEvent.type(searchInput, "launch-team");
    await userEvent.click(documentScope.getByText(/configure "launch-team"/i));

    await expect(canvas.getByText(/last create request: launch-team/i)).toBeInTheDocument();
    await expect(canvas.getByRole("combobox")).toHaveTextContent("production");
  },
};

export const CaseDistinctOptions: Story = {
  args: {
    suggestions: ["Prod", "prod"],
    options: [
      { groupName: "Prod", accountCount: 2, isPersisted: true },
      { groupName: "prod", accountCount: 1, isPersisted: true },
    ],
    formatAccountCountLabel: (count: number) => `${count} accounts`,
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const documentScope = within(canvasElement.ownerDocument.body);

    await userEvent.click(canvas.getByRole("combobox"));

    await expect(documentScope.getByText(/^Prod$/)).toBeInTheDocument();
    await expect(documentScope.getByText(/^prod$/)).toBeInTheDocument();

    const searchInput = documentScope.getByRole("textbox");
    await userEvent.clear(searchInput);
    await userEvent.type(searchInput, "PROD");
    await expect(documentScope.getByText(/configure "PROD"/i)).toBeInTheDocument();
  },
};
