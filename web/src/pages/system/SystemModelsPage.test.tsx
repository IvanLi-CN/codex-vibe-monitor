/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type { ModelsDevSyncPreview, SettingsPayload } from "../../lib/api";
import SystemModelsPage from "./SystemModelsPage";

const apiMocks = vi.hoisted(() => ({
  fetchSettings: vi.fn(),
  updatePricingSettings: vi.fn(),
  previewModelsDevPriceSync: vi.fn(),
  applyModelsDevPriceSync: vi.fn(),
  updateManagedModelPreset: vi.fn(),
  deleteManagedModel: vi.fn(),
}));

vi.mock("../../lib/api", async () => {
  const actual = await vi.importActual<typeof import("../../lib/api")>("../../lib/api");
  return {
    ...actual,
    fetchSettings: apiMocks.fetchSettings,
    updatePricingSettings: apiMocks.updatePricingSettings,
    previewModelsDevPriceSync: apiMocks.previewModelsDevPriceSync,
    applyModelsDevPriceSync: apiMocks.applyModelsDevPriceSync,
    updateManagedModelPreset: apiMocks.updateManagedModelPreset,
    deleteManagedModel: apiMocks.deleteManagedModel,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

function makeSettings(): SettingsPayload {
  return {
    proxy: {
      hijackEnabled: true,
      mergeUpstreamEnabled: false,
      fastModeRewriteMode: "disabled",
      upstream429MaxRetries: 0,
      websocketEnabled: false,
      upstreamWebsocketDefaultEnabled: false,
      requestBodyLoggingEnabled: false,
      responseBodyLoggingEnabled: false,
      encryptedSessionOwnerRoutingEnabled: false,
      defaultHijackEnabled: true,
      models: ["preset-without-price", "shared-model"],
      enabledModels: ["shared-model"],
    },
    forwardProxy: { message: "" },
    pricing: {
      catalogVersion: "test",
      entries: [
        {
          model: "priced-only-model",
          inputPer1m: 0.5,
          outputPer1m: 1.5,
          cacheInputPer1m: null,
          cacheReadPer1m: null,
          cacheWritePer1m: null,
          reasoningPer1m: null,
          source: "official",
        },
        {
          model: "custom-model",
          inputPer1m: 3,
          outputPer1m: 6,
          cacheInputPer1m: null,
          cacheReadPer1m: null,
          cacheWritePer1m: null,
          reasoningPer1m: null,
          source: "custom",
        },
      ],
    },
  };
}

function makePreview(): ModelsDevSyncPreview {
  return {
    fetchedAt: "2026-09-30T00:00:00Z",
    providerCount: 2,
    candidateCount: 4,
    providers: [
      { id: "provider-a", name: "Provider A", docUrl: "https://provider-a.example/docs" },
      { id: "provider-b", name: "Provider B", docUrl: "https://provider-b.example/docs" },
    ],
    candidates: [
      {
        model: "new-model",
        name: "New Model",
        providerId: "provider-a",
        providerName: "Provider A",
        docUrl: "https://provider-a.example/docs",
        inputPer1m: 1,
        outputPer1m: 2,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        unsupportedDimensions: ["image"],
        importable: true,
      },
      {
        model: "custom-model",
        name: "Custom Model",
        providerId: "provider-a",
        providerName: "Provider A",
        docUrl: "https://provider-a.example/docs",
        inputPer1m: 4,
        outputPer1m: 8,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        unsupportedDimensions: [],
        importable: true,
      },
      {
        model: "shared-model",
        name: "Shared Model A",
        providerId: "provider-a",
        providerName: "Provider A",
        docUrl: "https://provider-a.example/docs",
        inputPer1m: 2,
        outputPer1m: 4,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        unsupportedDimensions: [],
        importable: true,
      },
      {
        model: "shared-model",
        name: "Shared Model B",
        providerId: "provider-b",
        providerName: "Provider B",
        docUrl: "https://provider-b.example/docs",
        inputPer1m: 9,
        outputPer1m: 10,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        unsupportedDimensions: [],
        importable: true,
      },
    ],
  };
}

function renderPage() {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider>
        <SystemModelsPage />
      </I18nProvider>,
    );
  });
}

async function flushEffects() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

function clickButton(label: string) {
  const button = Array.from(document.body.querySelectorAll("button")).find((item) =>
    item.textContent?.includes(label),
  );
  expect(button).toBeTruthy();
  act(() => button?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

function clickModelAction(model: string, label: string) {
  const button = Array.from(document.body.querySelectorAll<HTMLButtonElement>("button")).find(
    (item) => {
      if (item.getAttribute("aria-label") !== label) return false;
      return item.closest("tr, article")?.textContent?.includes(model) ?? false;
    },
  );
  expect(button).toBeTruthy();
  act(() => button?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
}

function setInputValue(input: HTMLInputElement, value: string) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  expect(setter).toBeTruthy();
  act(() => {
    setter?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

function setLabeledInput(labelText: string, value: string) {
  const label = Array.from(document.body.querySelectorAll("label")).find(
    (item) => item.querySelector("span")?.textContent?.trim() === labelText,
  );
  const input = label?.querySelector("input");
  expect(input).toBeTruthy();
  setInputValue(input!, value);
}

describe("SystemModelsPage", () => {
  beforeEach(() => {
    window.localStorage.setItem("codex-vibe-monitor.locale", "zh");
    apiMocks.fetchSettings.mockResolvedValue(makeSettings());
    apiMocks.previewModelsDevPriceSync.mockResolvedValue(makePreview());
    apiMocks.applyModelsDevPriceSync.mockImplementation(async (entries) => ({
      catalogVersion: "test",
      entries,
    }));
    apiMocks.updatePricingSettings.mockResolvedValue(makeSettings().pricing);
    apiMocks.updateManagedModelPreset.mockResolvedValue(makeSettings().proxy);
    apiMocks.deleteManagedModel.mockResolvedValue({ deletedModel: "preset-without-price" });
  });

  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    window.localStorage.removeItem("codex-vibe-monitor.locale");
    Object.values(apiMocks).forEach((mock) => mock.mockReset());
  });

  it("shows the merged model directory and marks missing prices", async () => {
    renderPage();
    await flushEffects();

    const text = host?.textContent ?? "";
    expect(text).toContain("preset-without-price");
    expect(text).toContain("priced-only-model");
    expect(text).toContain("custom-model");
    expect(text).toContain("—");
  });

  it("shows the initial loading state until settings are available", async () => {
    let resolveSettings!: (settings: SettingsPayload) => void;
    apiMocks.fetchSettings.mockImplementationOnce(
      () => new Promise<SettingsPayload>((resolve) => (resolveSettings = resolve)),
    );

    renderPage();
    expect(host?.querySelector('[aria-busy="true"]')).toBeTruthy();
    expect(host?.textContent).not.toContain("preset-without-price");

    await act(async () => resolveSettings(makeSettings()));
    expect(host?.textContent).toContain("preset-without-price");
  });

  it("adds a manual price as a custom catalog entry", async () => {
    renderPage();
    await flushEffects();
    clickButton("新增模型");

    setLabeledInput("模型", "manual-new-model");
    setLabeledInput("输入 / 1M", "0.25");
    setLabeledInput("输出 / 1M", "0.75");
    setLabeledInput("缓存读取 / 1M", "0.1");
    clickButton("保存价格");
    await flushEffects();

    expect(apiMocks.updatePricingSettings).toHaveBeenCalledWith(
      expect.objectContaining({
        entries: expect.arrayContaining([
          expect.objectContaining({
            model: "manual-new-model",
            inputPer1m: 0.25,
            outputPer1m: 0.75,
            cacheReadPer1m: 0.1,
            source: "custom",
          }),
        ]),
      }),
    );
  });

  it("edits an existing price and saves it as a custom entry", async () => {
    renderPage();
    await flushEffects();
    clickModelAction("custom-model", "编辑价格");

    setLabeledInput("输入 / 1M", "4");
    setLabeledInput("输出 / 1M", "8");
    clickButton("保存价格");
    await flushEffects();

    expect(apiMocks.updatePricingSettings).toHaveBeenCalledWith(
      expect.objectContaining({
        entries: expect.arrayContaining([
          expect.objectContaining({
            model: "custom-model",
            inputPer1m: 4,
            outputPer1m: 8,
            source: "custom",
          }),
        ]),
      }),
    );
  });

  it("updates the proxy preset when its switch is enabled", async () => {
    renderPage();
    await flushEffects();
    const presetSwitch = document.body.querySelector<HTMLButtonElement>(
      'button[role="switch"][aria-label="在代理模型列表中启用 preset-without-price"]',
    );
    expect(presetSwitch?.getAttribute("aria-checked")).toBe("false");
    apiMocks.updateManagedModelPreset.mockResolvedValueOnce({
      ...makeSettings().proxy,
      enabledModels: ["preset-without-price", "shared-model"],
    });

    act(() => presetSwitch?.click());
    await flushEffects();

    expect(apiMocks.updateManagedModelPreset).toHaveBeenCalledWith("preset-without-price", true);
    expect(
      document.body
        .querySelector<HTMLButtonElement>(
          'button[role="switch"][aria-label="在代理模型列表中启用 preset-without-price"]',
        )
        ?.getAttribute("aria-checked"),
    ).toBe("true");
  });

  it("confirms deletion and removes the model through the delete API", async () => {
    renderPage();
    await flushEffects();
    clickModelAction("shared-model", "删除");
    await flushEffects();

    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog?.textContent).toContain("shared-model");
    expect(apiMocks.deleteManagedModel).not.toHaveBeenCalled();
    const confirmButton = Array.from(dialog?.querySelectorAll("button") ?? []).find(
      (button) => button.textContent?.trim() === "删除",
    );
    expect(confirmButton).toBeTruthy();
    act(() => confirmButton?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await flushEffects();

    expect(apiMocks.deleteManagedModel).toHaveBeenCalledWith("shared-model");
    expect(document.body.textContent).not.toContain("shared-model");
  });

  it("requires a provider choice and applies only the selected preview prices", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    expect(document.body.textContent ?? "").toContain("Provider A");
    expect(document.body.textContent ?? "").toContain("Provider B");
    expect(document.body.textContent ?? "").toContain("不导入");

    const priceCheckbox = (model: string) =>
      document.body.querySelector<HTMLInputElement>(`input[aria-label="同步 ${model} 的价格"]`);
    expect(priceCheckbox("new-model")?.checked).toBe(true);
    expect(priceCheckbox("custom-model")?.checked).toBe(false);
    expect(priceCheckbox("shared-model")?.checked).toBe(false);

    const providerTrigger = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="为 shared-model 选择一个供应商报价"]',
    );
    expect(providerTrigger).toBeTruthy();
    act(() => providerTrigger?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    const providerOption = Array.from(document.body.querySelectorAll('[role="option"]')).find(
      (option) => option.textContent?.includes("Provider B (provider-b)"),
    );
    expect(providerOption).toBeTruthy();
    act(() => providerOption?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await flushEffects();
    expect(priceCheckbox("shared-model")?.checked).toBe(true);

    const newModelCheckbox = priceCheckbox("new-model");
    expect(newModelCheckbox).toBeTruthy();
    act(() => newModelCheckbox?.click());
    await flushEffects();
    clickButton("同步所选");
    await flushEffects();

    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith([
      expect.objectContaining({
        model: "shared-model",
        inputPer1m: 9,
        outputPer1m: 10,
        source: "models.dev",
      }),
    ]);
  });

  it("preserves explicit price deselection when provider filters change", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const priceCheckbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    expect(priceCheckbox?.checked).toBe(true);
    act(() => priceCheckbox?.click());
    await flushEffects();
    expect(priceCheckbox?.checked).toBe(false);

    const providerBFilter = Array.from(
      document.body.querySelectorAll<HTMLInputElement>('details input[type="checkbox"]'),
    ).find((input) => input.parentElement?.textContent?.includes("provider-b"));
    expect(providerBFilter?.checked).toBe(true);
    act(() => providerBFilter?.click());
    await flushEffects();

    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 new-model 的价格"]')
        ?.checked,
    ).toBe(false);
    clickButton("同步所选");
    await flushEffects();
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith(
      expect.not.arrayContaining([expect.objectContaining({ model: "new-model" })]),
    );
  });

  it("keeps provider conflicts explicit when searching by provider", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const search = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="搜索供应商或模型"]',
    );
    expect(search).toBeTruthy();
    setInputValue(search!, "Provider A");
    await flushEffects();

    expect(
      document.body.querySelector<HTMLButtonElement>(
        'button[aria-label="为 shared-model 选择一个供应商报价"]',
      ),
    ).toBeTruthy();
    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 shared-model 的价格"]')
        ?.checked,
    ).toBe(false);

    clickButton("同步所选");
    await flushEffects();
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith(
      expect.not.arrayContaining([expect.objectContaining({ model: "shared-model" })]),
    );
  });

  it("applies selected candidates hidden by the search filter", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const newModelCheckbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    expect(newModelCheckbox?.checked).toBe(true);
    const search = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="搜索供应商或模型"]',
    );
    expect(search).toBeTruthy();
    setInputValue(search!, "shared-model");
    await flushEffects();
    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 new-model 的价格"]'),
    ).toBeNull();

    const providerTrigger = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="为 shared-model 选择一个供应商报价"]',
    );
    expect(providerTrigger).toBeTruthy();
    act(() => providerTrigger?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    const providerOption = Array.from(document.body.querySelectorAll('[role="option"]')).find(
      (option) => option.textContent?.includes("Provider B (provider-b)"),
    );
    expect(providerOption).toBeTruthy();
    act(() => providerOption?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
    await flushEffects();

    clickButton("同步所选");
    await flushEffects();
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith(
      expect.arrayContaining([
        expect.objectContaining({ model: "new-model", source: "models.dev" }),
        expect.objectContaining({
          model: "shared-model",
          inputPer1m: 9,
          outputPer1m: 10,
          source: "models.dev",
        }),
      ]),
    );
  });
});
