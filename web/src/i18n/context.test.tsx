/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { I18nProvider, useTranslation } from "./context";

let host: HTMLDivElement | null = null;
let root: Root | null = null;
const storageValues = new Map<string, string>();

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "localStorage", {
    configurable: true,
    value: {
      clear: () => storageValues.clear(),
      getItem: (key: string) => storageValues.get(key) ?? null,
      key: (index: number) => Array.from(storageValues.keys())[index] ?? null,
      get length() {
        return storageValues.size;
      },
      removeItem: (key: string) => storageValues.delete(key),
      setItem: (key: string, value: string) => storageValues.set(key, value),
    } satisfies Storage,
  });
});

function LocaleProbe() {
  const { locale, setLocale } = useTranslation();
  return (
    <button type="button" onClick={() => setLocale("en")}>
      {locale}
    </button>
  );
}

describe("I18nProvider", () => {
  afterEach(() => {
    act(() => {
      root?.unmount();
    });
    host?.remove();
    host = null;
    root = null;
    window.localStorage.clear();
  });

  it("supports a non-persistent locale for isolated stories", () => {
    window.localStorage.setItem("codex-vibe-monitor.locale", "zh");

    host = document.createElement("div");
    document.body.appendChild(host);
    root = createRoot(host);
    act(() => {
      root?.render(
        <I18nProvider initialLocale="zh" persistLocale={false}>
          <LocaleProbe />
        </I18nProvider>,
      );
    });

    const button = host.querySelector("button");
    expect(button?.textContent).toBe("zh");
    act(() => {
      button?.click();
    });

    expect(button?.textContent).toBe("en");
    expect(window.localStorage.getItem("codex-vibe-monitor.locale")).toBe("zh");
  });
});
