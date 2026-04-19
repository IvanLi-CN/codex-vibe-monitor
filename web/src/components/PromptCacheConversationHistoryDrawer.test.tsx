/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { PromptCacheConversationHistoryDrawer } from "./PromptCacheConversationHistoryDrawer";

const { historyMocks } = vi.hoisted(() => ({
  historyMocks: {
    usePromptCacheConversationHistory: vi.fn(),
  },
}));

vi.mock("../i18n", () => ({
  useTranslation: () => ({
    locale: "zh",
    t: (key: string, values?: Record<string, string | number>) => {
      if (values?.loaded != null && values?.total != null) {
        return `${key}:${values.loaded}/${values.total}`;
      }
      if (values?.count != null) return `${key}:${values.count}`;
      return key;
    },
  }),
}));

vi.mock("./prompt-cache-conversation-history-shared", () => ({
  PromptCacheConversationInvocationTable: ({
    records,
  }: {
    records: Array<{ invokeId?: string }>;
  }) => (
    <div data-testid="history-table">
      {records.map((record, index) => (
        <div key={record.invokeId ?? index}>{record.invokeId ?? "record"}</div>
      ))}
    </div>
  ),
  usePromptCacheConversationHistory:
    historyMocks.usePromptCacheConversationHistory,
}));

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
  vi.restoreAllMocks();
  historyMocks.usePromptCacheConversationHistory.mockReset();
});

function render(ui: React.ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("PromptCacheConversationHistoryDrawer", () => {
  it("keeps the live history drawer width constrained and wraps long keys", () => {
    historyMocks.usePromptCacheConversationHistory.mockReturnValue({
      visibleRecords: [],
      effectiveTotal: 0,
      loadedCount: 0,
      isLoading: false,
      error: null,
      hasHydrated: true,
    });

    const conversationKey =
      "prompt_cache_key_that_should_wrap_cleanly_in_the_drawer_header_because_it_has_no_natural_breakpoints";

    render(
      <PromptCacheConversationHistoryDrawer
        open
        conversationKey={conversationKey}
        onClose={() => undefined}
      />,
    );

    const drawerShell = document.body.querySelector(".drawer-shell");
    expect(drawerShell?.className).toContain("max-w-[78rem]");

    const heading = document.getElementById(
      drawerShell?.getAttribute("aria-labelledby") ?? "",
    );
    expect(heading?.textContent).toContain(conversationKey);
    expect(heading?.className).toContain("break-all");
  });
});
