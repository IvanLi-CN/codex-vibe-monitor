/** @vitest-environment jsdom */

import userEvent from "@testing-library/user-event";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import type {
  ModelsDevSyncMemoryPatch,
  ModelsDevSyncMemoryState,
  ModelsDevSyncPreview,
  SettingsPayload,
} from "../../lib/api";
import SystemModelsPage from "./SystemModelsPage";

const apiMocks = vi.hoisted(() => ({
  fetchSettings: vi.fn(),
  updatePricingSettings: vi.fn(),
  previewModelsDevPriceSync: vi.fn(),
  updateModelsDevSyncMemory: vi.fn(),
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
    updateModelsDevSyncMemory: apiMocks.updateModelsDevSyncMemory,
    applyModelsDevPriceSync: apiMocks.applyModelsDevPriceSync,
    updateManagedModelPreset: apiMocks.updateManagedModelPreset,
    deleteManagedModel: apiMocks.deleteManagedModel,
  };
});

let host: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  if (typeof globalThis.PointerEvent === "undefined") {
    Object.defineProperty(window, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
    Object.defineProperty(globalThis, "PointerEvent", {
      configurable: true,
      writable: true,
      value: MouseEvent,
    });
  }
  if (typeof HTMLElement.prototype.hasPointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "hasPointerCapture", {
      configurable: true,
      writable: true,
      value: () => false,
    });
  }
  if (typeof HTMLElement.prototype.setPointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "setPointerCapture", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
  if (typeof HTMLElement.prototype.releasePointerCapture !== "function") {
    Object.defineProperty(HTMLElement.prototype, "releasePointerCapture", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
  if (typeof HTMLElement.prototype.scrollIntoView !== "function") {
    Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
      configurable: true,
      writable: true,
      value: () => undefined,
    });
  }
});

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

function makeSyncMemory(): ModelsDevSyncMemoryState {
  return {
    catalogBaselineInitialized: true,
    providerSelectionInitialized: true,
    providerSelections: [
      { providerId: "provider-a", selected: true },
      { providerId: "provider-b", selected: true },
    ],
    modelSelections: [],
    quoteProviderChoices: [],
    unviewedModelIds: [],
  };
}

function applyMemoryPatch(
  current: ModelsDevSyncMemoryState,
  patch: ModelsDevSyncMemoryPatch,
): ModelsDevSyncMemoryState {
  const providers = new Map(current.providerSelections.map((item) => [item.providerId, item]));
  patch.providerSelections?.forEach((item) => providers.set(item.providerId, item));
  const selections = new Map(
    current.modelSelections.map((item) => [`${item.model}\0${item.providerId}`, item]),
  );
  patch.modelSelections?.forEach((item) =>
    selections.set(`${item.model}\0${item.providerId}`, item),
  );
  const quoteChoices = new Map(current.quoteProviderChoices.map((item) => [item.model, item]));
  patch.quoteProviderChoices?.forEach((item) => quoteChoices.set(item.model, item));
  const viewed = new Set(patch.viewedModelIds ?? []);
  return {
    ...current,
    providerSelections: Array.from(providers.values()),
    modelSelections: Array.from(selections.values()),
    quoteProviderChoices: Array.from(quoteChoices.values()),
    unviewedModelIds: current.unviewedModelIds.filter((model) => !viewed.has(model)),
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function makePreview(syncState = makeSyncMemory()): ModelsDevSyncPreview {
  return {
    fetchedAt: "2026-09-30T00:00:00Z",
    providerCount: 2,
    candidateCount: 4,
    providers: [
      {
        id: "provider-a",
        name: "Provider A",
        docUrl: "https://provider-a.example/docs",
      },
      {
        id: "provider-b",
        name: "Provider B",
        docUrl: "https://provider-b.example/docs",
      },
    ],
    syncState,
    candidates: [
      {
        model: "new-model",
        name: "New Model",
        providerId: "provider-a",
        providerName: "Provider A",
        docUrl: "https://provider-a.example/docs",
        status: null,
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
        status: null,
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
        status: null,
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
        status: null,
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

async function selectQuoteProvider(model: string, providerId: string) {
  const user = userEvent.setup();
  const trigger = document.body.querySelector<HTMLButtonElement>(
    `button[role="combobox"][aria-label="为 ${model} 选择一个供应商报价"]`,
  );
  expect(trigger).toBeTruthy();
  await user.click(trigger!);
  const option = Array.from(document.body.querySelectorAll<HTMLElement>('[role="option"]')).find(
    (item) => item.textContent?.includes(providerId),
  );
  expect(option).toBeTruthy();
  await user.click(option!);
}

let storedSyncMemory: ModelsDevSyncMemoryState;

describe("SystemModelsPage", () => {
  beforeEach(() => {
    window.localStorage.setItem("codex-vibe-monitor.locale", "zh");
    vi.spyOn(HTMLElement.prototype, "offsetHeight", "get").mockImplementation(function () {
      return this.getAttribute("aria-label") === "模型价格候选列表" ? 520 : 0;
    });
    storedSyncMemory = makeSyncMemory();
    apiMocks.fetchSettings.mockResolvedValue(makeSettings());
    apiMocks.previewModelsDevPriceSync.mockImplementation(async () =>
      makePreview(storedSyncMemory),
    );
    apiMocks.updateModelsDevSyncMemory.mockImplementation(
      async (patch: ModelsDevSyncMemoryPatch) => {
        storedSyncMemory = applyMemoryPatch(storedSyncMemory, patch);
        return storedSyncMemory;
      },
    );
    apiMocks.applyModelsDevPriceSync.mockImplementation(async (entries) => ({
      catalogVersion: "test",
      entries,
    }));
    apiMocks.updatePricingSettings.mockResolvedValue(makeSettings().pricing);
    apiMocks.updateManagedModelPreset.mockResolvedValue(makeSettings().proxy);
    apiMocks.deleteManagedModel.mockResolvedValue({
      deletedModel: "preset-without-price",
    });
  });

  afterEach(() => {
    act(() => root?.unmount());
    host?.remove();
    host = null;
    root = null;
    window.localStorage.removeItem("codex-vibe-monitor.locale");
    vi.restoreAllMocks();
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

  it("leaves local prices untouched when preview fails and allows retry", async () => {
    apiMocks.previewModelsDevPriceSync
      .mockRejectedValueOnce(new Error("models.dev retrieval failed"))
      .mockResolvedValueOnce(makePreview());

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const alert = document.body.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("无法获取 models.dev 目录。");
    expect(alert?.textContent).not.toContain("models.dev retrieval failed");
    expect(alert?.querySelector("button")?.textContent).toContain("重试");
    expect(apiMocks.applyModelsDevPriceSync).not.toHaveBeenCalled();
    expect(apiMocks.updatePricingSettings).not.toHaveBeenCalled();
    expect(host?.textContent).toContain("priced-only-model");

    clickButton("重试");
    await flushEffects();

    expect(apiMocks.previewModelsDevPriceSync).toHaveBeenCalledTimes(2);
    expect(document.body.textContent).toContain("Provider A");
    expect(apiMocks.applyModelsDevPriceSync).not.toHaveBeenCalled();
    expect(apiMocks.updatePricingSettings).not.toHaveBeenCalled();
    expect(host?.textContent).toContain("priced-only-model");
  });

  it("keeps the price-apply retry action inside the localized error alert", async () => {
    apiMocks.applyModelsDevPriceSync.mockRejectedValueOnce(
      new Error('Request failed: 502 {"message":"temporary failure"}'),
    );

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    clickButton("同步所选");
    await flushEffects();

    const alert = document.body.querySelector('[role="alert"]');
    expect(alert?.textContent).toContain("无法更新模型价格。");
    expect(alert?.textContent).not.toContain("502");
    expect(alert?.querySelector("button")?.textContent).toContain("重试");

    clickButton("重试");
    await flushEffects();

    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledTimes(2);
    expect(document.body.querySelector('[role="status"]')?.textContent).toContain("已更新");
  });

  it("ignores a preview response after its review was closed and reopened", async () => {
    const stalePreview = deferred<ModelsDevSyncPreview>();
    const latestMemory = makeSyncMemory();
    latestMemory.providerSelections = [
      { providerId: "provider-a", selected: false },
      { providerId: "provider-b", selected: true },
    ];
    apiMocks.previewModelsDevPriceSync
      .mockReturnValueOnce(stalePreview.promise)
      .mockResolvedValueOnce(makePreview(latestMemory));

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    expect(apiMocks.previewModelsDevPriceSync).toHaveBeenCalledTimes(1);

    const user = userEvent.setup();
    await user.keyboard("{Escape}");
    await flushEffects();
    expect(document.body.querySelector('[role="dialog"]')).toBeNull();
    const previewOptions = apiMocks.previewModelsDevPriceSync.mock.calls[0]?.[0] as
      | { signal: AbortSignal }
      | undefined;
    expect(previewOptions?.signal.aborted).toBe(true);

    clickButton("全部同步");
    await flushEffects();
    const providerPicker = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="筛选供应商"]',
    );
    expect(providerPicker?.textContent).toContain("1 / 2");

    await act(async () => {
      stalePreview.resolve(makePreview());
      await stalePreview.promise;
    });
    expect(providerPicker?.textContent).toContain("1 / 2");
  });

  it("keeps a saved selection when an older preview response arrives later", async () => {
    const savingMemory = deferred<ModelsDevSyncMemoryState>();
    const stalePreview = deferred<ModelsDevSyncPreview>();
    apiMocks.updateModelsDevSyncMemory.mockReturnValueOnce(savingMemory.promise);
    apiMocks.previewModelsDevPriceSync
      .mockResolvedValueOnce(makePreview())
      .mockReturnValueOnce(stalePreview.promise);

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    await flushEffects();
    expect(apiMocks.updateModelsDevSyncMemory).toHaveBeenCalledTimes(1);

    clickButton("取消");
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    expect(apiMocks.previewModelsDevPriceSync).toHaveBeenCalledTimes(2);

    const savedMemory = makeSyncMemory();
    savedMemory.modelSelections = [
      { model: "new-model", providerId: "provider-a", selected: true },
    ];
    await act(async () => {
      savingMemory.resolve(savedMemory);
      await savingMemory.promise;
    });
    await flushEffects();

    await act(async () => {
      stalePreview.resolve(makePreview(makeSyncMemory()));
      await stalePreview.promise;
    });
    await flushEffects();

    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 new-model 的价格"]')
        ?.checked,
    ).toBe(true);
  });

  it("keeps the latest preview state when an older memory patch response arrives later", async () => {
    const savingMemory = deferred<ModelsDevSyncMemoryState>();
    const latestMemory = makeSyncMemory();
    latestMemory.providerSelections = [
      { providerId: "provider-a", selected: false },
      { providerId: "provider-b", selected: true },
    ];
    apiMocks.updateModelsDevSyncMemory.mockReturnValueOnce(savingMemory.promise);
    apiMocks.previewModelsDevPriceSync
      .mockResolvedValueOnce(makePreview())
      .mockResolvedValueOnce(makePreview(latestMemory));

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    await flushEffects();
    clickButton("取消");
    await flushEffects();

    clickButton("全部同步");
    await flushEffects();
    const providerPicker = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="筛选供应商"]',
    );
    expect(providerPicker?.textContent).toContain("1 / 2");

    const olderMemory = makeSyncMemory();
    olderMemory.modelSelections = [
      { model: "new-model", providerId: "provider-a", selected: true },
    ];
    await act(async () => {
      savingMemory.resolve(olderMemory);
      await savingMemory.promise;
    });
    await flushEffects();

    expect(providerPicker?.textContent).toContain("1 / 2");
  });

  it("keeps the review open until an in-flight price apply finishes", async () => {
    const applying = deferred<SettingsPayload["pricing"]>();
    apiMocks.applyModelsDevPriceSync.mockReturnValueOnce(applying.promise);

    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    await flushEffects();
    clickButton("同步所选");
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledTimes(1);

    const providerPicker = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="筛选供应商"]',
    );
    const showDeprecated = document.body.querySelector<HTMLButtonElement>(
      'button[role="switch"][aria-label="显示已弃用报价"]',
    );
    const selectAll = Array.from(document.body.querySelectorAll<HTMLButtonElement>("button")).find(
      (button) => button.getAttribute("aria-label") === "全选",
    );
    const quoteProvider = document.body.querySelector<HTMLButtonElement>(
      'button[role="combobox"][aria-label="为 shared-model 选择一个供应商报价"]',
    );
    expect(providerPicker?.disabled).toBe(true);
    expect(showDeprecated?.disabled).toBe(true);
    expect(selectAll?.disabled).toBe(true);
    expect(quoteProvider?.disabled).toBe(true);
    expect(checkbox?.disabled).toBe(true);

    const user = userEvent.setup();
    const cancelButton = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent?.includes("取消"));
    expect(cancelButton?.disabled).toBe(true);
    await user.click(cancelButton!);
    expect(document.body.querySelector('[role="dialog"]')).toBeTruthy();

    await user.keyboard("{Escape}");
    const dialog = document.body.querySelector<HTMLElement>('[role="dialog"]');
    expect(dialog).toBeTruthy();
    expect(dialog?.querySelector<HTMLButtonElement>("button[aria-label]")?.disabled).toBe(true);

    await act(async () => {
      applying.resolve(makeSettings().pricing);
      await applying.promise;
    });
    await flushEffects();
    expect(document.body.querySelector('[role="dialog"]')).toBeTruthy();
    expect(document.body.textContent).toContain("已更新 1 个模型价格");
  });

  it("keeps failed selection-memory retry available after price apply succeeds", async () => {
    apiMocks.updateModelsDevSyncMemory.mockRejectedValueOnce(new Error("memory write failed"));
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    await flushEffects();
    expect(document.body.textContent).toContain("选择记忆未保存");
    expect(document.body.textContent).not.toContain("memory write failed");

    clickButton("同步所选");
    await flushEffects();
    expect(document.body.textContent).toContain("已更新 1 个模型价格");
    expect(document.body.textContent).toContain("选择记忆未保存");
    expect(document.body.textContent).not.toContain("memory write failed");
    clickButton("重试");
    await flushEffects();

    expect(document.body.textContent).not.toContain("选择记忆未保存");
    expect(storedSyncMemory.modelSelections).toContainEqual({
      model: "new-model",
      providerId: "provider-a",
      selected: true,
    });
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

  it("keeps quote-provider selections separate and applies only checked prices", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    expect(document.body.textContent ?? "").toContain("Provider A");
    expect(document.body.textContent ?? "").toContain("不导入");

    const priceCheckbox = (model: string) =>
      document.body.querySelector<HTMLInputElement>(`input[aria-label="同步 ${model} 的价格"]`);
    expect(priceCheckbox("new-model")?.checked).toBe(false);
    expect(priceCheckbox("custom-model")?.checked).toBe(false);
    expect(priceCheckbox("shared-model")?.checked).toBe(false);

    await selectQuoteProvider("shared-model", "provider-b");
    await flushEffects();
    expect(document.body.textContent ?? "").toContain("Provider B");
    expect(priceCheckbox("shared-model")?.checked).toBe(false);
    expect(apiMocks.updateModelsDevSyncMemory).toHaveBeenCalledWith(
      expect.objectContaining({
        quoteProviderChoices: [{ model: "shared-model", providerId: "provider-b" }],
      }),
    );

    act(() => priceCheckbox("shared-model")?.click());
    await flushEffects();
    expect(priceCheckbox("shared-model")?.checked).toBe(true);
    await selectQuoteProvider("shared-model", "provider-a");
    await flushEffects();
    expect(priceCheckbox("shared-model")?.checked).toBe(false);
    await selectQuoteProvider("shared-model", "provider-b");
    await flushEffects();
    expect(priceCheckbox("shared-model")?.checked).toBe(true);

    const newModelCheckbox = priceCheckbox("new-model");
    expect(newModelCheckbox).toBeTruthy();
    act(() => newModelCheckbox?.click());
    await flushEffects();
    clickButton("同步所选");
    await flushEffects();

    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledTimes(1);
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith([
      {
        model: "new-model",
        inputPer1m: 1,
        outputPer1m: 2,
        cacheInputPer1m: null,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "models.dev",
      },
      {
        model: "shared-model",
        inputPer1m: 9,
        outputPer1m: 10,
        cacheInputPer1m: null,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "models.dev",
      },
    ]);
  });

  it("keeps remembered provider choices unresolved when provider filters hide them", async () => {
    storedSyncMemory.quoteProviderChoices = [{ model: "shared-model", providerId: "provider-b" }];
    storedSyncMemory.modelSelections = [
      { model: "shared-model", providerId: "provider-b", selected: true },
    ];
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const providerTrigger = document.body.querySelector<HTMLButtonElement>(
      'button[aria-label="筛选供应商"]',
    );
    expect(providerTrigger).toBeTruthy();
    act(() => providerTrigger?.click());
    const providerSearch = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="搜索供应商 ID 或名称"]',
    );
    expect(providerSearch).toBeTruthy();
    setInputValue(providerSearch!, "Provider B");
    clickButton("清空");
    await flushEffects();

    const priceCheckbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 shared-model 的价格"]',
    );
    expect(priceCheckbox?.disabled).toBe(true);
    expect(document.body.textContent).toContain("已记忆的供应商在当前筛选中不可用");
    await selectQuoteProvider("shared-model", "provider-a");
    await flushEffects();
    expect(priceCheckbox?.disabled).toBe(false);
    expect(priceCheckbox?.checked).toBe(false);
    expect(storedSyncMemory.modelSelections).toContainEqual({
      model: "shared-model",
      providerId: "provider-b",
      selected: true,
    });
  });

  it("applies selected candidates hidden by model search and reports the hidden count", async () => {
    storedSyncMemory.modelSelections = [
      { model: "new-model", providerId: "provider-a", selected: true },
      { model: "shared-model", providerId: "provider-b", selected: true },
    ];
    storedSyncMemory.quoteProviderChoices = [{ model: "shared-model", providerId: "provider-b" }];
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const search = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="搜索模型名称或 ID"]',
    );
    expect(search).toBeTruthy();
    setInputValue(search!, "shared-model");
    await flushEffects();

    expect(document.body.textContent).toContain("1 个待同步价格被模型搜索隐藏");
    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 shared-model 的价格"]')
        ?.checked,
    ).toBe(true);

    clickButton("同步所选");
    await flushEffects();
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledTimes(1);
    expect(apiMocks.applyModelsDevPriceSync).toHaveBeenCalledWith([
      {
        model: "new-model",
        inputPer1m: 1,
        outputPer1m: 2,
        cacheInputPer1m: null,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "models.dev",
      },
      {
        model: "shared-model",
        inputPer1m: 9,
        outputPer1m: 10,
        cacheInputPer1m: null,
        cacheReadPer1m: null,
        cacheWritePer1m: null,
        reasoningPer1m: null,
        source: "models.dev",
      },
    ]);
  });

  it("saves checkbox changes immediately and restores them after cancel", async () => {
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();

    const newModelCheckbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    expect(newModelCheckbox?.checked).toBe(false);
    act(() => newModelCheckbox?.click());
    await flushEffects();
    expect(storedSyncMemory.modelSelections).toContainEqual({
      model: "new-model",
      providerId: "provider-a",
      selected: true,
    });
    clickButton("取消");
    await flushEffects();
    expect(apiMocks.applyModelsDevPriceSync).not.toHaveBeenCalled();

    clickButton("全部同步");
    await flushEffects();
    expect(
      document.body.querySelector<HTMLInputElement>('input[aria-label="同步 new-model 的价格"]')
        ?.checked,
    ).toBe(true);
  });

  it("keeps optimistic changes and offers a retry when saving memory fails", async () => {
    apiMocks.updateModelsDevSyncMemory.mockRejectedValueOnce(new Error("memory write failed"));
    renderPage();
    await flushEffects();
    clickButton("全部同步");
    await flushEffects();
    const checkbox = document.body.querySelector<HTMLInputElement>(
      'input[aria-label="同步 new-model 的价格"]',
    );
    act(() => checkbox?.click());
    await flushEffects();
    expect(checkbox?.checked).toBe(true);
    expect(document.body.textContent).toContain("选择记忆未保存");
    expect(document.body.textContent).not.toContain("memory write failed");
    const alert = document.body.querySelector('[role="alert"]');
    const retryButton = Array.from(
      document.body.querySelectorAll<HTMLButtonElement>("button"),
    ).find((button) => button.textContent?.includes("重试"));
    expect(retryButton?.closest('[role="alert"]')).toBe(alert);
    clickButton("重试");
    await flushEffects();
    expect(document.body.textContent).not.toContain("选择记忆未保存");
    expect(storedSyncMemory.modelSelections).toContainEqual({
      model: "new-model",
      providerId: "provider-a",
      selected: true,
    });
  });
});
