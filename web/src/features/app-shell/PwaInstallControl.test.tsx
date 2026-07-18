/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PwaInstallControl, type PwaInstallControlLabels } from "./PwaInstallControl";

const labels: PwaInstallControlLabels = {
  promptButton: "安装应用",
  laterButton: "稍后",
  manualButton: "添加到主屏幕",
  installedButton: "已安装",
  switcherAria: "打开安装应用入口",
  closeButton: "关闭",
  closeAria: "关闭安装说明",
  shellReady: "离线壳已就绪",
  shellPending: "离线壳待完成",
  offlineChip: "当前离线",
  promptTitle: "安装 Codex Vibe Monitor",
  promptDescription: "安装为独立应用窗口。",
  promptHint: "离线时仍可打开壳层。",
  manualTitle: "将 Codex Vibe Monitor 添加到主屏幕",
  manualDescription: "iPhone / iPad 上的 Safari 不会弹原生安装提示。",
  manualStepOpenShare: "打开分享菜单。",
  manualStepAdd: "选择添加到主屏幕。",
  manualStepConfirm: "确认图标名称后完成添加。",
  installedTitle: "应用已安装",
  installedDescription: "当前浏览器已经运行在独立壳层里。",
  installedHint: "离线时仍可打开壳层，实时数据会在恢复联网后继续更新。",
};

let host: HTMLDivElement | null = null;
let root: Root | null = null;

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
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("PwaInstallControl", () => {
  it("renders the install prompt as a centered modal without any trigger button", async () => {
    const onPromptInstall = vi.fn(async () => undefined);

    render(
      <PwaInstallControl
        mode="prompt"
        open
        onOpenChange={() => undefined}
        shellReady
        isOffline={false}
        labels={labels}
        onPromptInstall={onPromptInstall}
      />,
    );

    expect(host?.querySelector('[data-testid="pwa-install-control"]')).toBeNull();

    const dialog = document.body.querySelector('[role="dialog"]') as HTMLElement | null;
    expect(dialog).toBeInstanceOf(HTMLElement);
    expect(dialog?.textContent).toContain("安装 Codex Vibe Monitor");
    expect(dialog?.textContent).toContain("稍后");
    expect(dialog?.textContent).toContain("安装应用");

    const classTokens = new Set(dialog?.className.split(/\s+/).filter(Boolean));
    expect(classTokens.has("left-1/2")).toBe(true);
    expect(classTokens.has("top-1/2")).toBe(true);
    expect(classTokens.has("-translate-x-1/2")).toBe(true);
    expect(classTokens.has("-translate-y-1/2")).toBe(true);
    expect(classTokens.has("rounded-[1.75rem]")).toBe(true);

    const confirm = document.body.querySelector(
      '[data-testid="pwa-install-confirm"]',
    ) as HTMLButtonElement | null;
    expect(confirm).toBeInstanceOf(HTMLButtonElement);

    await act(async () => {
      confirm?.click();
      await Promise.resolve();
    });

    expect(onPromptInstall).toHaveBeenCalledTimes(1);
  });
});
