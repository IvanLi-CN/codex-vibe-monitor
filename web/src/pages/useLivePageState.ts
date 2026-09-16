import { useState } from "react";
import type { TranslationKey } from "../i18n";
import type { PromptCacheConversationSelection } from "../lib/api";

export const LIMIT_OPTIONS = [20, 50, 100];
export const LIVE_TABS = ["conversations", "records", "routing", "proxy"] as const;
export type LiveTab = (typeof LIVE_TABS)[number];
export const LIVE_TAB_IDS: Record<LiveTab, { tab: string; panel: string }> = {
  conversations: {
    tab: "live-workspace-tab-conversations",
    panel: "live-workspace-panel-conversations",
  },
  records: {
    tab: "live-workspace-tab-records",
    panel: "live-workspace-panel-records",
  },
  routing: {
    tab: "live-workspace-tab-routing",
    panel: "live-workspace-panel-routing",
  },
  proxy: {
    tab: "live-workspace-tab-proxy",
    panel: "live-workspace-panel-proxy",
  },
};
export const DEFAULT_PROMPT_CACHE_SELECTION: PromptCacheConversationSelection = {
  mode: "count",
  limit: 50,
};
export const PROMPT_CACHE_SELECTION_OPTIONS: Array<
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      labelKey: TranslationKey;
      count: number;
    }
  | {
      value: string;
      selection: PromptCacheConversationSelection;
      labelKey: TranslationKey;
      hours: number;
    }
> = [
  {
    value: "count:20",
    selection: { mode: "count", limit: 20 },
    labelKey: "live.conversations.option.count",
    count: 20,
  },
  {
    value: "count:50",
    selection: { mode: "count", limit: 50 },
    labelKey: "live.conversations.option.count",
    count: 50,
  },
  {
    value: "count:100",
    selection: { mode: "count", limit: 100 },
    labelKey: "live.conversations.option.count",
    count: 100,
  },
  {
    value: "activityWindow:1",
    selection: { mode: "activityWindow", activityHours: 1 },
    labelKey: "live.conversations.option.activityHours",
    hours: 1,
  },
  {
    value: "activityWindow:3",
    selection: { mode: "activityWindow", activityHours: 3 },
    labelKey: "live.conversations.option.activityHours",
    hours: 3,
  },
  {
    value: "activityWindow:6",
    selection: { mode: "activityWindow", activityHours: 6 },
    labelKey: "live.conversations.option.activityHours",
    hours: 6,
  },
  {
    value: "activityWindow:12",
    selection: { mode: "activityWindow", activityHours: 12 },
    labelKey: "live.conversations.option.activityHours",
    hours: 12,
  },
  {
    value: "activityWindow:24",
    selection: { mode: "activityWindow", activityHours: 24 },
    labelKey: "live.conversations.option.activityHours",
    hours: 24,
  },
];
export const PROMPT_CACHE_SELECTION_LOOKUP = new Map(
  PROMPT_CACHE_SELECTION_OPTIONS.map((option) => [option.value, option.selection]),
);
export const SUMMARY_WINDOWS: { value: string; labelKey: TranslationKey }[] = [
  { value: "current", labelKey: "live.summary.current" },
  { value: "30m", labelKey: "live.summary.30m" },
  { value: "1h", labelKey: "live.summary.1h" },
  { value: "1d", labelKey: "live.summary.1d" },
];

const PROMPT_CACHE_SELECTION_STORAGE_KEY = "codex-vibe-monitor.live.prompt-cache-selection";
const LIVE_TAB_STORAGE_KEY = "codex-vibe-monitor.live.active-tab";

function readStoredValue(key: string, fallback: string, isValid: (value: string) => boolean) {
  if (typeof window === "undefined") return fallback;
  try {
    const value = window.localStorage.getItem(key);
    return value && isValid(value) ? value : fallback;
  } catch {
    return fallback;
  }
}

function persistStoredValue(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    // Local storage is optional; the live view remains usable without it.
  }
}

function readPromptCacheSelectionValue() {
  return readStoredValue(PROMPT_CACHE_SELECTION_STORAGE_KEY, "count:50", (value) =>
    PROMPT_CACHE_SELECTION_LOOKUP.has(value),
  );
}

function readLiveTab() {
  return readStoredValue(LIVE_TAB_STORAGE_KEY, "routing", (value) =>
    LIVE_TABS.includes(value as LiveTab),
  ) as LiveTab;
}

export interface LivePageState {
  limit: number;
  setLimit: (limit: number) => void;
  activeTab: LiveTab;
  setActiveTab: (tab: LiveTab) => void;
  conversationSelectionValue: string;
  setConversationSelectionValue: (value: string) => void;
  expandedPromptCacheKeys: string[];
  syncExpandedPromptCacheKeys: (visibleKeys: string[]) => void;
  toggleExpandedPromptCacheKey: (key: string) => void;
  toggleAllVisiblePromptCacheKeys: (visibleKeys: string[]) => void;
  summaryWindow: string;
  setSummaryWindow: (window: string) => void;
  routingWindow: "15m" | "1h" | "6h" | "24h";
  setRoutingWindow: (window: "15m" | "1h" | "6h" | "24h") => void;
}

export function useLivePageState(): LivePageState {
  const [limit, setLimit] = useState(50);
  const [activeTab, setActiveTabState] = useState<LiveTab>(readLiveTab);
  const [conversationSelectionValue, setConversationSelectionValueState] = useState(
    readPromptCacheSelectionValue,
  );
  const [expandedPromptCacheKeys, setExpandedPromptCacheKeys] = useState<string[]>([]);
  const [summaryWindow, setSummaryWindow] = useState("current");
  const [routingWindow, setRoutingWindow] = useState<LivePageState["routingWindow"]>("1h");

  const setActiveTab = (tab: LiveTab) => {
    setActiveTabState(tab);
    persistStoredValue(LIVE_TAB_STORAGE_KEY, tab);
  };
  const setConversationSelectionValue = (value: string) => {
    if (!PROMPT_CACHE_SELECTION_LOOKUP.has(value)) return;
    setConversationSelectionValueState(value);
    persistStoredValue(PROMPT_CACHE_SELECTION_STORAGE_KEY, value);
  };
  const syncExpandedPromptCacheKeys = (visibleKeys: string[]) => {
    const visible = new Set(visibleKeys);
    setExpandedPromptCacheKeys((current) => {
      const next = current.filter((key) => visible.has(key));
      return next.length === current.length ? current : next;
    });
  };
  const toggleExpandedPromptCacheKey = (key: string) => {
    setExpandedPromptCacheKeys((current) =>
      current.includes(key) ? current.filter((value) => value !== key) : [...current, key],
    );
  };
  const toggleAllVisiblePromptCacheKeys = (visiblePromptCacheKeys: string[]) => {
    setExpandedPromptCacheKeys((current) => {
      const allExpanded = visiblePromptCacheKeys.every((key) => current.includes(key));
      return allExpanded
        ? current.filter((key) => !visiblePromptCacheKeys.includes(key))
        : [
            ...current.filter((key) => !visiblePromptCacheKeys.includes(key)),
            ...visiblePromptCacheKeys,
          ];
    });
  };

  return {
    limit,
    setLimit,
    activeTab,
    setActiveTab,
    conversationSelectionValue,
    setConversationSelectionValue,
    expandedPromptCacheKeys,
    syncExpandedPromptCacheKeys,
    toggleExpandedPromptCacheKey,
    toggleAllVisiblePromptCacheKeys,
    summaryWindow,
    setSummaryWindow,
    routingWindow,
    setRoutingWindow,
  };
}
