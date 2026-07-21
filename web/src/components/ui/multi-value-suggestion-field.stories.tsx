import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { MultiValueSuggestionField } from "./multi-value-suggestion-field";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-2xl">{children}</div>
    </div>
  );
}

const inputClassName =
  "h-9 w-full rounded-md border border-base-300/80 bg-base-100 px-3 text-sm text-base-content shadow-sm outline-none transition focus-visible:ring-2 focus-visible:ring-primary focus-visible:ring-offset-2 focus-visible:ring-offset-base-100 disabled:cursor-not-allowed disabled:opacity-60";

function ControlledMultiValueSuggestionField({
  label,
  inputLabel,
  options,
  placeholder,
  initialValues = [],
  error,
  surface,
}: {
  label: string;
  inputLabel: string;
  options: Array<string | { value: string; label?: string; searchText?: string }>;
  placeholder?: string;
  initialValues?: string[];
  error?: string;
  surface?: "default" | "embedded";
}) {
  const [values, setValues] = useState(initialValues);
  const [inputValue, setInputValue] = useState("");

  return (
    <StorySurface>
      <div className="space-y-4">
        <MultiValueSuggestionField
          label={label}
          inputLabel={inputLabel}
          values={values}
          onValuesChange={setValues}
          inputValue={inputValue}
          onInputValueChange={setInputValue}
          options={options}
          placeholder={placeholder}
          emptyText="No matches"
          addLabel="Add"
          error={error}
          inputClassName={inputClassName}
          surface={surface}
        />
        <div className="rounded-xl border border-base-300/70 bg-base-100/45 px-4 py-3 text-sm text-base-content/70">
          Current values:{" "}
          <span className="font-mono text-base-content">
            {values.length > 0 ? values.join(", ") : "—"}
          </span>
        </div>
      </div>
    </StorySurface>
  );
}

const meta = {
  title: "UI/MultiValueSuggestionField",
  component: MultiValueSuggestionField,
} satisfies Meta<typeof MultiValueSuggestionField>;

export default meta;

type Story = StoryObj<Record<string, never>>;

export const Basic: Story = {
  render: () => (
    <ControlledMultiValueSuggestionField
      label="Model"
      inputLabel="Model"
      placeholder="Select models"
      options={["gpt-5.4", "gpt-5", "gpt-5-mini", "gpt-4o-mini"]}
    />
  ),
};

export const LabeledOptions: Story = {
  render: () => (
    <ControlledMultiValueSuggestionField
      label="Upstream account"
      inputLabel="Upstream account"
      placeholder="Search name or ID"
      options={[
        { value: "42", label: "Pool Alpha (#42)", searchText: "Pool Alpha 42" },
        { value: "77", label: "Pool Beta (#77)", searchText: "Pool Beta 77" },
        {
          value: "105",
          label: "Nightly QA (#105)",
          searchText: "Nightly QA 105",
        },
      ]}
      initialValues={["42"]}
    />
  ),
};

export const ErrorState: Story = {
  render: () => (
    <ControlledMultiValueSuggestionField
      label="Model"
      inputLabel="Model"
      placeholder="Select models"
      options={["gpt-5.4", "gpt-5", "gpt-5-mini"]}
      initialValues={["gpt-5.4"]}
      error="Select at least one model before adding reasoning effort."
    />
  ),
};

export const EmbeddedSelector: Story = {
  render: () => (
    <ControlledMultiValueSuggestionField
      label="Model"
      inputLabel="Model"
      placeholder="Select models"
      options={["gpt-5.4", "gpt-5", "gpt-5-mini", "gpt-4o-mini"]}
      initialValues={["gpt-5.4"]}
      surface="embedded"
    />
  ),
};
