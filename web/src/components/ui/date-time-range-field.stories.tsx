import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { DateTimeRangeField } from "./date-time-range-field";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-base-200 px-6 py-6 text-base-content">
      <div className="mx-auto w-full max-w-xl">{children}</div>
    </div>
  );
}

const rangeOptions = [
  { value: "today", label: "Today" },
  { value: "1d", label: "Past 24 hours" },
  { value: "7d", label: "Past 7 days" },
  { value: "30d", label: "Past 30 days" },
  { value: "custom", label: "Custom range" },
] as const;

function ControlledDateTimeRangeField() {
  const [value, setValue] = useState({
    preset: "today" as (typeof rangeOptions)[number]["value"],
    from: "2026-07-18T00:00",
    to: "2026-07-18T13:45",
  });

  return (
    <StorySurface>
      <DateTimeRangeField
        label="Time range"
        customPresetValue="custom"
        value={value}
        options={rangeOptions.map((option) => ({ ...option }))}
        summary={
          value.preset === "custom"
            ? `${value.from.replace("T", " ")} - ${value.to.replace("T", " ")}`
            : rangeOptions.find((option) => option.value === value.preset)?.label
        }
        fromLabel="From"
        toLabel="To"
        onChange={setValue}
      />
    </StorySurface>
  );
}

const meta = {
  title: "UI/DateTimeRangeField",
  component: DateTimeRangeField,
} satisfies Meta<typeof DateTimeRangeField>;

export default meta;

type Story = StoryObj<Record<string, never>>;

export const Default: Story = {
  render: () => <ControlledDateTimeRangeField />,
};

export const InvalidOrder: Story = {
  render: () => (
    <StorySurface>
      <DateTimeRangeField
        label="Time range"
        customPresetValue="custom"
        value={{
          preset: "custom",
          from: "2026-07-18T15:00",
          to: "2026-07-18T14:00",
        }}
        options={rangeOptions.map((option) => ({ ...option }))}
        summary="2026-07-18 15:00 - 2026-07-18 14:00"
        fromLabel="From"
        toLabel="To"
        error="End time must be after start time."
        onChange={() => {}}
      />
    </StorySurface>
  ),
};
