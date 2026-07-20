import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { NumericRangeField } from "./numeric-range-field";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-xl">{children}</div>
    </div>
  );
}

function ControlledNumericRangeField() {
  const [value, setValue] = useState({
    minValue: "2400",
    maxValue: "6400",
  });

  return (
    <StorySurface>
      <NumericRangeField
        label="Total tokens"
        sliderMin={0}
        sliderMax={12000}
        minAriaLabel="Minimum total tokens"
        maxAriaLabel="Maximum total tokens"
        unitLabel="TOKENS"
        step={1}
        minValue={value.minValue}
        maxValue={value.maxValue}
        onChange={setValue}
      />
    </StorySurface>
  );
}

const meta = {
  title: "UI/NumericRangeField",
  component: NumericRangeField,
} satisfies Meta<typeof NumericRangeField>;

export default meta;

type Story = StoryObj<Record<string, never>>;

export const Default: Story = {
  render: () => <ControlledNumericRangeField />,
};

export const WithValidationError: Story = {
  render: () => (
    <StorySurface>
      <NumericRangeField
        label="Total duration"
        sliderMin={0}
        sliderMax={4000}
        minAriaLabel="Minimum total duration"
        maxAriaLabel="Maximum total duration"
        unitLabel="MS"
        step={0.1}
        minValue="3200"
        maxValue="800"
        error="Total duration range must be in ascending order."
        onChange={() => {}}
      />
    </StorySurface>
  ),
};

export const EmbeddedInSection: Story = {
  render: () => (
    <StorySurface>
      <section className="rounded-2xl border border-base-300/70 bg-base-100/35 p-4">
        <div className="grid gap-4 md:grid-cols-2">
          <NumericRangeField
            label="Total tokens"
            surface="embedded"
            sliderMin={0}
            sliderMax={12000}
            minAriaLabel="Minimum total tokens"
            maxAriaLabel="Maximum total tokens"
            unitLabel="TOKENS"
            step={1}
            minValue="2400"
            maxValue="6400"
            onChange={() => {}}
          />
          <NumericRangeField
            label="Total duration"
            surface="embedded"
            sliderMin={0}
            sliderMax={31620}
            minAriaLabel="Minimum total duration"
            maxAriaLabel="Maximum total duration"
            unitLabel="MS"
            step={0.1}
            minValue=""
            maxValue=""
            onChange={() => {}}
          />
        </div>
      </section>
    </StorySurface>
  ),
};
