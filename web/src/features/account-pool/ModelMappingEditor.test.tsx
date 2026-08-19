/** @vitest-environment jsdom */
import { act, type ReactNode, useRef, useState } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelMapping } from "../../lib/api";
import {
  areModelMappingDraftsEqual,
  type ModelMappingDraft,
  ModelMappingEditor,
  toModelMappings,
  validateModelMappingDrafts,
} from "./ModelMappingEditor";

let host: HTMLDivElement | null = null;
let root: Root | null = null;

const initialMappings: ModelMappingDraft[] = [
  { id: "first", sourceModel: "client-*", targetModel: "upstream-a", enabled: true },
  { id: "second", sourceModel: "gpt-5", targetModel: "upstream-b", enabled: false },
];

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  host = null;
  root = null;
  vi.restoreAllMocks();
});

function render(ui: ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

function inputByName(name: string) {
  const input = document.querySelector(`input[name="${name}"]`);
  if (!(input instanceof HTMLInputElement)) throw new Error(`missing input ${name}`);
  return input;
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  if (!setter) throw new Error("missing native input setter");
  act(() => {
    setter.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
    input.dispatchEvent(new Event("change", { bubbles: true }));
  });
}

function click(button: HTMLElement) {
  act(() => {
    button.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
}

function EditorHarness({
  initial = [],
  baseline = [],
  onSave = vi.fn(),
}: {
  initial?: ModelMappingDraft[];
  baseline?: ModelMappingDraft[];
  onSave?: (mappings: ModelMapping[]) => void;
}) {
  const [mappings, setMappings] = useState(initial);
  const [savedBaseline, setSavedBaseline] = useState(baseline);
  const nextId = useRef(0);
  return (
    <I18nProvider>
      <ModelMappingEditor
        mappings={mappings}
        availableModelOptions={["gpt-5", "gpt-5-mini", "upstream-a"]}
        dirty={!areModelMappingDraftsEqual(mappings, savedBaseline)}
        onChange={setMappings}
        onSave={() => {
          onSave(toModelMappings(mappings));
          setSavedBaseline(mappings);
        }}
        createId={() => {
          nextId.current += 1;
          return `new-${nextId.current}`;
        }}
      />
    </I18nProvider>
  );
}

describe("ModelMappingEditor", () => {
  it("allows custom source and target models, validates drafts, and saves independently", () => {
    const onSave = vi.fn();
    render(<EditorHarness onSave={onSave} />);

    const add = Array.from(document.querySelectorAll("button")).find((button) =>
      /add mapping|新增映射/i.test(button.textContent ?? ""),
    );
    if (!(add instanceof HTMLButtonElement)) throw new Error("missing add mapping button");
    click(add);

    const save = document.querySelector('[data-testid="model-mapping-save"]');
    if (!(save instanceof HTMLButtonElement)) throw new Error("missing mapping save button");
    expect(save.disabled).toBe(true);
    expect(document.body.textContent).toMatch(
      /Every mapping needs an original and target model.|每条映射都需要填写原模型和目标模型。/,
    );

    setInputValue(inputByName("modelMappingSource-new-1"), " client-custom-* ");
    setInputValue(inputByName("modelMappingTarget-new-1"), " upstream-custom ");
    expect(save.disabled).toBe(false);

    const enabled = document.querySelector('[role="switch"]');
    if (!(enabled instanceof HTMLButtonElement)) throw new Error("missing mapping switch");
    click(enabled);
    expect(enabled.getAttribute("data-state")).toBe("unchecked");

    click(save);
    expect(onSave).toHaveBeenCalledWith([
      { sourceModel: "client-custom-*", targetModel: "upstream-custom", enabled: false },
    ]);
    expect(save.disabled).toBe(true);
  });

  it("supports dnd-kit keyboard reordering and row deletion without changing mapping values", async () => {
    render(<EditorHarness initial={initialMappings} baseline={[]} />);

    const reorderButtons = Array.from(
      document.querySelectorAll(
        'button[aria-label="Reorder mapping"], button[aria-label="调整映射顺序"]',
      ),
    );
    const desktopReorderButtons = reorderButtons.filter((button) =>
      button.className.includes("desktop:inline-flex"),
    );
    const secondReorderButton = desktopReorderButtons[1];
    if (!(secondReorderButton instanceof HTMLButtonElement)) {
      throw new Error("missing second reorder button");
    }
    await act(async () => {
      secondReorderButton.dispatchEvent(
        new KeyboardEvent("keydown", { code: "Space", key: " ", bubbles: true }),
      );
    });
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { code: "ArrowUp", key: "ArrowUp", bubbles: true }),
      );
    });
    await act(async () => {
      document.dispatchEvent(
        new KeyboardEvent("keydown", { code: "Space", key: " ", bubbles: true }),
      );
    });

    const sourceModels = Array.from(
      document.querySelectorAll('input[name^="modelMappingSource-"]'),
    ).map((input) => (input as HTMLInputElement).value);
    expect(sourceModels).toEqual(["gpt-5", "client-*"]);

    const removeButtons = Array.from(
      document.querySelectorAll(
        'button[aria-label="Remove mapping"], button[aria-label="删除映射"]',
      ),
    );
    const remove = removeButtons[0];
    if (!(remove instanceof HTMLButtonElement)) throw new Error("missing remove button");
    click(remove);

    expect(document.querySelectorAll('input[name^="modelMappingSource-"]')).toHaveLength(1);
    expect(inputByName("modelMappingSource-first").value).toBe("client-*");
  });

  it("treats ASCII-case-insensitive duplicate sources as invalid while retaining disabled rows", () => {
    const drafts: ModelMappingDraft[] = [
      { id: "one", sourceModel: "GPT-*", targetModel: "a", enabled: true },
      { id: "two", sourceModel: "gpt-*", targetModel: "b", enabled: false },
    ];
    expect(validateModelMappingDrafts(drafts)).toBe("duplicate");
    expect(toModelMappings(drafts)[1]).toEqual({
      sourceModel: "gpt-*",
      targetModel: "b",
      enabled: false,
    });
  });
});
