import type { Meta, StoryObj } from "@storybook/react-vite";
import { useState } from "react";
import { expect, userEvent, waitFor, within } from "storybook/test";
import { InvocationModelFilterField } from "./invocation-model-filter-field";

function StorySurface({ children }: { children: React.ReactNode }) {
  return (
    <div className="min-h-screen bg-[#b0bfd0] px-8 py-8 text-base-content">
      <div className="mx-auto w-full max-w-4xl">
        <div
          data-testid="invocation-model-filter-story-surface"
          className="rounded-[28px] border border-[#d5e0ee] bg-white p-8"
        >
          {children}
        </div>
      </div>
    </div>
  );
}

const zhCopy = {
  label: "模型",
  hint: "使用标签选择器多选模型，并可选追加推理强度；还可以切换按请求侧或响应侧模型筛选。",
  modelLabel: "模型",
  reasoningEffortLabel: "推理强度",
  modelTargetLabel: "匹配侧",
  requestTargetLabel: "请求",
  responseTargetLabel: "响应",
  reroutedLabel: "重路由",
  reroutedAllLabel: "全部",
  reroutedOnlyLabel: "已重路由",
  notReroutedLabel: "未重路由",
  modelPlaceholder: "选择模型",
  reasoningEffortPlaceholder: "选择推理强度",
  emptyText: "无匹配项",
  loadingText: "搜索中…",
  addLabel: "添加",
  invalidError: "选择推理强度前必须至少选择一个模型。",
} as const;

function ControlledInvocationModelFilterField({
  initialValue = {
    modelTarget: "request" as const,
    modelRerouted: "all" as const,
    models: ["gpt-5.4"],
    reasoningEfforts: ["high"],
  },
  label = zhCopy.label,
}: {
  initialValue?: {
    modelTarget: "request" | "response";
    modelRerouted: "all" | "rerouted" | "notRerouted";
    models: string[];
    reasoningEfforts: string[];
  };
  label?: React.ReactNode;
}) {
  const [value, setValue] = useState(initialValue);
  const [modelInputValue, setModelInputValue] = useState("");
  const [reasoningInputValue, setReasoningInputValue] = useState("");

  return (
    <StorySurface>
      <InvocationModelFilterField
        label={label}
        hint={zhCopy.hint}
        value={value}
        onChange={setValue}
        modelLabel={zhCopy.modelLabel}
        reasoningEffortLabel={zhCopy.reasoningEffortLabel}
        modelTargetLabel={zhCopy.modelTargetLabel}
        requestTargetLabel={zhCopy.requestTargetLabel}
        responseTargetLabel={zhCopy.responseTargetLabel}
        reroutedLabel={zhCopy.reroutedLabel}
        reroutedAllLabel={zhCopy.reroutedAllLabel}
        reroutedOnlyLabel={zhCopy.reroutedOnlyLabel}
        notReroutedLabel={zhCopy.notReroutedLabel}
        modelInputValue={modelInputValue}
        onModelInputValueChange={setModelInputValue}
        modelOptions={["gpt-5.4", "gpt-5", "gpt-5-mini", "gpt-4o-mini"]}
        modelPlaceholder={zhCopy.modelPlaceholder}
        reasoningEffortInputValue={reasoningInputValue}
        onReasoningEffortInputValueChange={setReasoningInputValue}
        reasoningEffortOptions={["minimal", "low", "medium", "high", "xhigh"]}
        reasoningEffortPlaceholder={zhCopy.reasoningEffortPlaceholder}
        emptyText={zhCopy.emptyText}
        loadingText={zhCopy.loadingText}
        addLabel={zhCopy.addLabel}
        testId="invocation-model-filter-field"
      />
    </StorySurface>
  );
}

const meta = {
  title: "UI/InvocationModelFilterField",
  component: InvocationModelFilterField,
} satisfies Meta<typeof InvocationModelFilterField>;

export default meta;

type Story = StoryObj<Record<string, never>>;

export const Default: Story = {
  render: () => <ControlledInvocationModelFilterField />,
};

export const ResponseReroutedConfigured: Story = {
  render: () => (
    <ControlledInvocationModelFilterField
      initialValue={{
        modelTarget: "response",
        modelRerouted: "rerouted",
        models: ["gpt-5.4", "gpt-5-mini"],
        reasoningEfforts: ["high", "medium"],
      }}
    />
  ),
};

export const EvidenceRequestConfigured: Story = {
  render: () => (
    <ControlledInvocationModelFilterField
      label={<span className="sr-only">{zhCopy.label}</span>}
      initialValue={{
        modelTarget: "request",
        modelRerouted: "all",
        models: ["gpt-5.4", "gpt-5"],
        reasoningEfforts: ["medium"],
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByTestId("invocation-model-filter-field-trigger"),
    );

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="invocation-model-filter-field-panel"]'),
      ).not.toBeNull();
    });
  },
};

export const EvidenceResponseRerouted: Story = {
  render: () => (
    <ControlledInvocationModelFilterField
      label={<span className="sr-only">{zhCopy.label}</span>}
      initialValue={{
        modelTarget: "response",
        modelRerouted: "rerouted",
        models: ["gpt-5.4", "gpt-5-mini"],
        reasoningEfforts: ["high", "medium"],
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByTestId("invocation-model-filter-field-trigger"),
    );

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="invocation-model-filter-field-panel"]'),
      ).not.toBeNull();
    });
  },
};

export const EvidenceResponseNotRerouted: Story = {
  render: () => (
    <ControlledInvocationModelFilterField
      label={<span className="sr-only">{zhCopy.label}</span>}
      initialValue={{
        modelTarget: "response",
        modelRerouted: "notRerouted",
        models: ["gpt-5.4"],
        reasoningEfforts: ["medium"],
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    await userEvent.click(
      within(canvasElement).getByTestId("invocation-model-filter-field-trigger"),
    );

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="invocation-model-filter-field-panel"]'),
      ).not.toBeNull();
    });
  },
};

export const InvalidWithoutModel: Story = {
  render: () => (
    <StorySurface>
      <InvocationModelFilterField
        label={zhCopy.label}
        hint={zhCopy.hint}
        value={{
          modelTarget: "response",
          modelRerouted: "rerouted",
          models: [],
          reasoningEfforts: ["high"],
        }}
        onChange={() => {}}
        modelLabel={zhCopy.modelLabel}
        reasoningEffortLabel={zhCopy.reasoningEffortLabel}
        modelTargetLabel={zhCopy.modelTargetLabel}
        requestTargetLabel={zhCopy.requestTargetLabel}
        responseTargetLabel={zhCopy.responseTargetLabel}
        reroutedLabel={zhCopy.reroutedLabel}
        reroutedAllLabel={zhCopy.reroutedAllLabel}
        reroutedOnlyLabel={zhCopy.reroutedOnlyLabel}
        notReroutedLabel={zhCopy.notReroutedLabel}
        modelInputValue=""
        onModelInputValueChange={() => {}}
        modelOptions={["gpt-5.4", "gpt-5", "gpt-5-mini"]}
        modelPlaceholder={zhCopy.modelPlaceholder}
        reasoningEffortInputValue=""
        onReasoningEffortInputValueChange={() => {}}
        reasoningEffortOptions={["minimal", "low", "medium", "high", "xhigh"]}
        reasoningEffortPlaceholder={zhCopy.reasoningEffortPlaceholder}
        emptyText={zhCopy.emptyText}
        loadingText={zhCopy.loadingText}
        addLabel={zhCopy.addLabel}
        error={zhCopy.invalidError}
        testId="invocation-model-filter-field"
      />
    </StorySurface>
  ),
};
