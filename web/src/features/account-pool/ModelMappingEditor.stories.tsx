import type { Meta, StoryObj } from "@storybook/react-vite";
import { useRef, useState } from "react";
import { expect, userEvent, within } from "storybook/test";
import { I18nProvider } from "../../i18n";
import type { ModelMappingDraft } from "./ModelMappingEditor";
import {
  areModelMappingDraftsEqual,
  ModelMappingEditor,
  toModelMappings,
} from "./ModelMappingEditor";

const initialMappings: ModelMappingDraft[] = [
  {
    id: "story-client-fast",
    sourceModel: "client-fast-*",
    targetModel: "gpt-5.4-mini",
    enabled: true,
  },
  {
    id: "story-legacy",
    sourceModel: "legacy-pro",
    targetModel: "o3-mini",
    enabled: false,
  },
];

function MappingEditorStory() {
  const [mappings, setMappings] = useState(initialMappings);
  const [baseline, setBaseline] = useState(initialMappings);
  const [saveCount, setSaveCount] = useState(0);
  const nextId = useRef(0);
  const dirty = !areModelMappingDraftsEqual(mappings, baseline);

  return (
    <I18nProvider>
      <main className="min-h-screen bg-base-200 px-4 py-5 text-base-content desktop:px-10 desktop:py-8">
        <div className="mx-auto w-full max-w-6xl">
          <ModelMappingEditor
            mappings={mappings}
            availableModelOptions={[
              "gpt-5.4-mini",
              "gpt-5.4",
              "gpt-5.5",
              "o3-mini",
              "claude-sonnet-4",
            ]}
            dirty={dirty}
            onChange={setMappings}
            onSave={() => {
              setBaseline(mappings);
              setSaveCount((count) => count + 1);
            }}
            createId={() => {
              nextId.current += 1;
              return `story-new-${nextId.current}`;
            }}
          />
          <output className="sr-only" data-testid="model-mapping-save-count">
            {saveCount}
          </output>
          <output className="sr-only" data-testid="model-mapping-last-save">
            {JSON.stringify(toModelMappings(baseline))}
          </output>
        </div>
      </main>
    </I18nProvider>
  );
}

const meta = {
  title: "Account Pool/Components/Model Mapping Editor",
  component: MappingEditorStory,
  tags: ["autodocs", "test"],
  parameters: {
    layout: "fullscreen",
  },
} satisfies Meta<typeof MappingEditorStory>;

export default meta;
type Story = StoryObj<typeof meta>;

export const DesktopRoutingMappings: Story = {
  parameters: {
    viewport: { defaultViewport: "desktop1440x1024" },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("model-mapping-editor")).toBeVisible();
    await expect(canvas.getAllByRole("combobox")).toHaveLength(4);

    await userEvent.click(canvas.getByRole("button", { name: /add mapping|新增映射/i }));
    const fields = canvas.getAllByRole("combobox");
    await userEvent.type(fields[4], "custom-client-model");
    await userEvent.type(fields[5], "custom-upstream-model");
    await userEvent.click(canvas.getAllByRole("switch")[2]);
    await userEvent.click(canvas.getByTestId("model-mapping-save"));

    await expect(canvas.getByTestId("model-mapping-save-count")).toHaveTextContent("1");
    await expect(canvas.getByTestId("model-mapping-last-save")).toHaveTextContent(
      "custom-upstream-model",
    );
  },
};

export const MobileRoutingMappings: Story = {
  parameters: {
    viewport: { defaultViewport: "mobile393" },
  },
};
