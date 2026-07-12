/** @vitest-environment jsdom */
import type { ComponentProps } from "react";
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import type { ForwardProxyBindingNode } from "../../lib/api";
import { UpstreamAccountGroupNoteDialog } from "./UpstreamAccountGroupNoteDialog";

class MockResizeObserver {
  observe() {}
  unobserve() {}
  disconnect() {}
}

type DialogProps = ComponentProps<typeof UpstreamAccountGroupNoteDialog>;

let host: HTMLDivElement | null = null;
let overlayRoot: HTMLDivElement | null = null;
let root: Root | null = null;

beforeAll(() => {
  Object.defineProperty(globalThis, "IS_REACT_ACT_ENVIRONMENT", {
    configurable: true,
    writable: true,
    value: true,
  });
  Object.defineProperty(window, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(globalThis, "ResizeObserver", {
    configurable: true,
    writable: true,
    value: MockResizeObserver,
  });
  Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
    configurable: true,
    writable: true,
    value: () => undefined,
  });
});

afterEach(() => {
  act(() => {
    root?.unmount();
  });
  host?.remove();
  overlayRoot?.remove();
  host = null;
  overlayRoot = null;
  root = null;
});

function renderDialog(props: Partial<DialogProps> = {}) {
  host = document.createElement("div");
  overlayRoot = document.createElement("div");
  document.body.appendChild(host);
  document.body.appendChild(overlayRoot);
  root = createRoot(host);

  const defaultNodes: ForwardProxyBindingNode[] = [
    {
      key: "__direct__",
      source: "direct",
      displayName: "Direct",
      protocolLabel: "DIRECT",
      penalized: false,
      selectable: true,
      last24h: [],
    },
    {
      key: "fpn_7f1080a2fdb3a4d1",
      source: "manual",
      displayName: "JP Edge 01",
      protocolLabel: "HTTP",
      penalized: false,
      selectable: true,
      last24h: [],
    },
  ];

  const defaults: DialogProps = {
    open: true,
    container: overlayRoot,
    groupName: "production",
    note: "Premium routing",
    existing: true,
    busy: false,
    error: null,
    boundProxyKeys: [],
    singleAccountRotationEnabled: false,
    upstream429RetryEnabled: false,
    upstream429MaxRetries: 0,
    availableProxyNodes: defaultNodes,
    onNoteChange: () => undefined,
    onBoundProxyKeysChange: () => undefined,
    onSingleAccountRotationEnabledChange: () => undefined,
    onUpstream429RetryEnabledChange: () => undefined,
    onUpstream429MaxRetriesChange: () => undefined,
    onClose: () => undefined,
    onSave: () => undefined,
    title: "Edit group settings",
    existingDescription: "Existing group",
    draftDescription: "Draft group",
    noteLabel: "Group note",
    notePlaceholder: "Note",
    cancelLabel: "Cancel",
    saveLabel: "Save",
    closeLabel: "Close",
    existingBadgeLabel: "Persisted group",
    draftBadgeLabel: "Draft group",
    upstream429RetryLabel: "Upstream 429 retry",
    upstream429RetryHint:
      "Retry the same account after upstream 429 with a random delay.",
    upstream429RetryToggleLabel: "Retry the same account after upstream 429",
    upstream429RetryCountLabel: "Retry count",
    upstream429RetryCountOptions: [
      { value: 1, label: "1 retry" },
      { value: 2, label: "2 retries" },
      { value: 3, label: "3 retries" },
    ],
    singleAccountRotationLabel: "Single-account rotation load",
    singleAccountRotationHint:
      "Successful conversations stay on the same account until upstream 429 retry is exhausted.",
    singleAccountRotationToggleLabel:
      "Keep conversations on one account until final 429",
    proxyBindingsLabel: "Bound proxy nodes",
    proxyBindingsHint: "Leave empty to keep automatic routing.",
    proxyBindingsAutomaticLabel:
      "No nodes bound. This group uses automatic routing.",
    proxyBindingsLoadingLabel: "Loading proxy nodes…",
    proxyBindingsEmptyLabel: "No proxy nodes available.",
    proxyBindingsMissingLabel: "Missing",
    proxyBindingsUnavailableLabel: "Unavailable",
    proxyBindingsChartLabel: "24h request trend",
    proxyBindingsChartSuccessLabel: "Success",
    proxyBindingsChartFailureLabel: "Failure",
    proxyBindingsChartEmptyLabel: "No 24h data",
    proxyBindingsChartTotalLabel: "Total requests",
    proxyBindingsChartAriaLabel: "Last 24h request volume chart",
    proxyBindingsChartInteractionHint: "Hover or tap for details.",
    proxyBindingsChartLocaleTag: "en-US",
  };

  act(() => {
    root?.render(<UpstreamAccountGroupNoteDialog {...defaults} {...props} />);
  });
}

function bodyText() {
  return document.body.textContent ?? "";
}

function clickTab(label: RegExp) {
  const tab = Array.from(document.querySelectorAll('[role="tab"]')).find(
    (candidate) => label.test(candidate.textContent ?? ""),
  ) as HTMLButtonElement | undefined;
  expect(tab).toBeDefined();
  act(() => {
    tab?.click();
  });
}

describe("UpstreamAccountGroupNoteDialog", () => {
  it("shows protocol badges, keeps direct available, and never renders raw subscription URLs", () => {
    renderDialog({
      boundProxyKeys: ["__direct__"],
      groupName: "latam",
      note: "",
      availableProxyNodes: [
        {
          key: "__direct__",
          source: "direct",
          displayName: "Direct",
          protocolLabel: "DIRECT",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "fpn_vless_stable_key",
          source: "subscription",
          displayName: "Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "fpn_drain_stable_key",
          source: "manual",
          displayName: "Drain Node",
          protocolLabel: "HTTP",
          penalized: true,
          selectable: false,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    const text = bodyText();
    expect(text).toContain("Direct");
    expect(text).toContain("DIRECT");
    expect(text).toContain("VLESS");
    expect(text).not.toContain("vless://");

    const scrollRegion = document.querySelector(
      '[data-testid="proxy-binding-options-scroll-region"]',
    ) as HTMLElement | null;
    expect(scrollRegion).not.toBeNull();
    expect(scrollRegion?.className).toContain("overflow-y-auto");

    const dialog = document.querySelector(
      '[role="dialog"]',
    ) as HTMLElement | null;
    expect(dialog).not.toBeNull();
    expect(dialog?.className).not.toContain("max-w-[72rem]");
    expect(dialog?.className).toContain("desktop:max-w-[44rem]");

    const truncatedTitle = document.querySelector(
      '[title="Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS"]',
    ) as HTMLElement | null;
    expect(truncatedTitle).not.toBeNull();
    expect(truncatedTitle?.className).toContain("truncate");
  });

  it("adds identity hints for duplicate and missing bindings without exposing stored keys", () => {
    renderDialog({
      groupName: "overflow",
      note: "",
      boundProxyKeys: ["shared-edge-a", "legacy-missing-binding"],
      availableProxyNodes: [
        {
          key: "shared-edge-a",
          source: "subscription",
          displayName: "Shared Edge",
          protocolLabel: "HTTP",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "shared-edge-b",
          source: "subscription",
          displayName: "Shared Edge",
          protocolLabel: "HTTP",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "legacy-missing-binding",
          source: "missing",
          displayName: "Legacy Missing Binding",
          protocolLabel: "UNKNOWN",
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    const text = bodyText();
    expect(text).not.toContain("legacy-missing-binding");
    expect(text).toContain("Legacy Missing Binding");

    const identityHints = Array.from(
      document.querySelectorAll('[title^="ID "]'),
    );
    expect(identityHints.length).toBeGreaterThanOrEqual(2);
    expect(text).toContain("Missing");
  });

  it("shows visible identity hints for long truncated node names even when labels are unique", () => {
    renderDialog({
      groupName: "overflow",
      note: "",
      boundProxyKeys: ["edge-long-a"],
      availableProxyNodes: [
        {
          key: "edge-long-a",
          source: "subscription",
          displayName: "ivan-hinet-vless-vision-west-region-priority-a1",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "edge-long-b",
          source: "subscription",
          displayName: "ivan-hinet-vless-vision-west-region-priority-b9",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: true,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    const identityHints = Array.from(
      document.querySelectorAll('[title^="ID "]'),
    );
    expect(identityHints.length).toBeGreaterThanOrEqual(2);
  });

  it("renders restored non-ASCII display names for unavailable bound nodes without falling back to raw keys", () => {
    renderDialog({
      boundProxyKeys: ["fpn_deadbeefcafebabe"],
      availableProxyNodes: [
        {
          key: "fpn_deadbeefcafebabe",
          source: "missing",
          displayName: "东京专线 A",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("东京专线 A");
    expect(bodyText()).toContain("Unavailable");
    expect(bodyText()).not.toContain("fpn_deadbeefcafebabe");
  });

  it("falls back to the raw key only when no display metadata is available", () => {
    renderDialog({
      boundProxyKeys: ["fpn_missing_only"],
      availableProxyNodes: [],
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("fpn_missing_only");
    expect(bodyText()).toContain("Missing");
  });

  it("shows a loading placeholder instead of the empty-state message while the proxy catalog is still hydrating", () => {
    renderDialog({
      availableProxyNodes: [],
      proxyBindingsCatalogKind: "loading",
      proxyBindingsCatalogFreshness: "missing",
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("Loading proxy nodes…");
    expect(bodyText()).not.toContain("No proxy nodes available.");

    const loadingState = document.querySelector(
      '[data-testid="proxy-binding-options-loading"]',
    );
    expect(loadingState).not.toBeNull();
  });

  it("keeps bound missing proxy rows visible while the catalog is still hydrating", () => {
    const onBoundProxyKeysChange = vi.fn();

    renderDialog({
      boundProxyKeys: ["fpn_missing_only"],
      availableProxyNodes: [],
      onBoundProxyKeysChange,
      proxyBindingsCatalogKind: "loading",
      proxyBindingsCatalogFreshness: "missing",
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("Loading proxy nodes…");
    expect(bodyText()).toContain("fpn_missing_only");
    expect(bodyText()).toContain("Missing");

    const missingBindingButton = Array.from(
      document.querySelectorAll("button"),
    ).find((candidate) =>
      (candidate.textContent ?? "").includes("fpn_missing_only"),
    ) as HTMLButtonElement | undefined;

    expect(missingBindingButton).toBeDefined();

    act(() => {
      missingBindingButton?.click();
    });

    expect(onBoundProxyKeysChange).toHaveBeenCalledWith([]);
  });

  it("treats a missing proxy catalog as unresolved instead of showing the empty-state copy", () => {
    renderDialog({
      availableProxyNodes: [],
      proxyBindingsCatalogKind: "missing",
      proxyBindingsCatalogFreshness: "missing",
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("Loading proxy nodes…");
    expect(bodyText()).not.toContain("No proxy nodes available.");
  });

  it("blocks saving when every selected binding is unavailable", () => {
    renderDialog({
      boundProxyKeys: ["fpn_unavailable_only"],
      availableProxyNodes: [
        {
          key: "fpn_unavailable_only",
          source: "missing",
          displayName: "Drain Node",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain(
      "Select at least one available proxy node or clear bindings before saving.",
    );

    const saveButton = Array.from(document.querySelectorAll("button")).find(
      (candidate) => /save/i.test(candidate.textContent ?? ""),
    ) as HTMLButtonElement | undefined;

    expect(saveButton).toBeDefined();
    expect(saveButton?.disabled).toBe(true);
  });

  it("treats legacy alias bindings as selectable and canonicalizes them before saving", () => {
    const onBoundProxyKeysChange = vi.fn();

    renderDialog({
      boundProxyKeys: ["fpn_legacy_vless_alias"],
      onBoundProxyKeysChange,
      availableProxyNodes: [
        {
          key: "fpb_canonical_vless_key",
          aliasKeys: ["fpn_legacy_vless_alias"],
          source: "subscription",
          displayName: "东京专线 A",
          protocolLabel: "VLESS",
          penalized: false,
          selectable: true,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("东京专线 A");
    expect(bodyText()).not.toContain(
      "Select at least one available proxy node or clear bindings before saving.",
    );

    const saveButton = Array.from(document.querySelectorAll("button")).find(
      (candidate) => /save/i.test(candidate.textContent ?? ""),
    ) as HTMLButtonElement | undefined;

    expect(saveButton).toBeDefined();
    expect(saveButton?.disabled).toBe(false);
    expect(onBoundProxyKeysChange).toHaveBeenCalledWith([
      "fpb_canonical_vless_key",
    ]);
  });

  it("hides unrelated stale missing nodes from other groups", () => {
    renderDialog({
      boundProxyKeys: ["fpn_selected_node"],
      availableProxyNodes: [
        {
          key: "fpn_selected_node",
          source: "manual",
          displayName: "JP Edge 01",
          protocolLabel: "HTTP",
          penalized: false,
          selectable: true,
          last24h: [],
        },
        {
          key: "fpn_other_group_stale",
          source: "missing",
          displayName: "别组遗留节点",
          protocolLabel: "UNKNOWN",
          penalized: false,
          selectable: false,
          last24h: [],
        },
      ],
    });
    clickTab(/proxy nodes/i);

    expect(bodyText()).toContain("JP Edge 01");
    expect(bodyText()).not.toContain("别组遗留节点");
  });

  it("renders upstream 429 retry as a single 0..5 selector where 0 means off", () => {
    const onUpstream429RetryEnabledChange = vi.fn();
    const onUpstream429MaxRetriesChange = vi.fn();
    renderDialog({
      upstream429RetryEnabled: false,
      upstream429MaxRetries: 0,
      onUpstream429RetryEnabledChange,
      onUpstream429MaxRetriesChange,
    });
    clickTab(/routing settings/i);

    expect(
      document.querySelector(
        '[role="switch"][aria-label="Retry the same account after upstream 429"]',
      ),
    ).toBeNull();
    expect(
      document.querySelector('[role="combobox"][aria-label="Retry count"]'),
    ).toBeNull();

    const retryGroup = document.querySelector(
      '[role="radiogroup"][aria-label="Upstream 429 retry"]',
    ) as HTMLElement | null;
    expect(retryGroup).not.toBeNull();
    const retryOptions = Array.from(
      retryGroup?.querySelectorAll<HTMLButtonElement>('[role="radio"]') ?? [],
    );
    expect(retryOptions.map((option) => option.textContent)).toEqual([
      "0",
      "1",
      "2",
      "3",
      "4",
      "5",
    ]);
    expect(retryOptions[0]?.getAttribute("aria-checked")).toBe("true");

    act(() => {
      retryOptions[2]?.click();
    });

    expect(onUpstream429RetryEnabledChange).toHaveBeenCalledWith(true);
    expect(onUpstream429MaxRetriesChange).toHaveBeenCalledWith(2);
  });

  it("selects 0 and emits no-retry payload when clearing upstream 429 retry", () => {
    const onUpstream429RetryEnabledChange = vi.fn();
    const onUpstream429MaxRetriesChange = vi.fn();
    renderDialog({
      upstream429RetryEnabled: true,
      upstream429MaxRetries: 3,
      onUpstream429RetryEnabledChange,
      onUpstream429MaxRetriesChange,
    });
    clickTab(/routing settings/i);

    const retryGroup = document.querySelector(
      '[role="radiogroup"][aria-label="Upstream 429 retry"]',
    ) as HTMLElement | null;
    expect(retryGroup).not.toBeNull();
    const retryOptions = Array.from(
      retryGroup?.querySelectorAll<HTMLButtonElement>('[role="radio"]') ?? [],
    );
    expect(retryOptions[3]?.getAttribute("aria-checked")).toBe("true");

    act(() => {
      retryOptions[0]?.click();
    });

    expect(onUpstream429RetryEnabledChange).toHaveBeenCalledWith(false);
    expect(onUpstream429MaxRetriesChange).toHaveBeenCalledWith(0);
  });

  it("renders the single-account rotation switch and preserves its checked state", () => {
    const onSingleAccountRotationEnabledChange = vi.fn();
    renderDialog({
      singleAccountRotationEnabled: true,
      onSingleAccountRotationEnabledChange,
    });

    expect(bodyText()).toContain("Single-account rotation load");
    expect(bodyText()).toContain(
      "Successful conversations stay on the same account until upstream 429 retry is exhausted.",
    );

    const toggle = document.querySelector(
      '[role="switch"][aria-label="Keep conversations on one account until final 429"]',
    ) as HTMLElement | null;
    expect(toggle).not.toBeNull();
    expect(toggle?.getAttribute("aria-checked")).toBe("true");

    act(() => {
      toggle?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onSingleAccountRotationEnabledChange).toHaveBeenCalledWith(false);
  });

  it("blocks saving when node shunt is enabled without any bound node", () => {
    renderDialog({
      boundProxyKeys: [],
      nodeShuntEnabled: true,
      nodeShuntLabel: "Node shunt strategy",
      nodeShuntHint: "Each selected node becomes an exclusive slot.",
      nodeShuntToggleLabel: "Enable node shunt strategy",
      nodeShuntWarning:
        "Enable this strategy only after binding at least one node (including Direct).",
    });

    const text = bodyText();
    expect(text).toContain("Node shunt strategy");
    expect(text).toContain(
      "Enable this strategy only after binding at least one node (including Direct).",
    );

    const toggle = document.querySelector(
      '[role="switch"][aria-label="Enable node shunt strategy"]',
    ) as HTMLElement | null;
    expect(toggle?.getAttribute("aria-checked")).toBe("true");

    const saveButton = Array.from(document.querySelectorAll("button")).find(
      (candidate) => /save/i.test(candidate.textContent ?? ""),
    ) as HTMLButtonElement | undefined;
    expect(saveButton).toBeDefined();
    expect(saveButton?.disabled).toBe(true);
  });

  it("allows saving when node shunt is enabled with only unavailable bound nodes", () => {
    renderDialog({
      boundProxyKeys: ["drain-only"],
      nodeShuntEnabled: true,
      nodeShuntLabel: "Node shunt strategy",
      nodeShuntHint: "Each selected node becomes an exclusive slot.",
      nodeShuntToggleLabel: "Enable node shunt strategy",
      nodeShuntWarning:
        "Enable this strategy only after binding at least one node (including Direct).",
      availableProxyNodes: [
        {
          key: "drain-only",
          source: "manual",
          displayName: "Drain Only",
          protocolLabel: "HTTP",
          penalized: true,
          selectable: false,
          last24h: [],
        },
      ],
    });

    expect(bodyText()).not.toContain(
      "Enable this strategy only after binding at least one node (including Direct).",
    );

    const saveButton = Array.from(document.querySelectorAll("button")).find(
      (candidate) => /save/i.test(candidate.textContent ?? ""),
    ) as HTMLButtonElement | undefined;
    expect(saveButton).toBeDefined();
    expect(saveButton?.disabled).toBe(false);
  });

  it("shows a blocked-delete popover on click instead of rendering helper text below the footer buttons", () => {
    const onDelete = vi.fn();
    renderDialog({
      accountCount: 4,
      onDelete,
      deleteLabel: "Delete group",
      deleteDisabledHint:
        "Move the remaining 4 account(s) out before deleting this group.",
    });

    expect(bodyText()).not.toContain(
      "Move the remaining 4 account(s) out before deleting this group.",
    );

    const deleteButton = Array.from(document.querySelectorAll("button")).find(
      (candidate) => /delete group/i.test(candidate.textContent ?? ""),
    ) as HTMLButtonElement | undefined;
    expect(deleteButton).toBeDefined();
    expect(deleteButton?.disabled).toBe(false);
    expect(deleteButton?.getAttribute("aria-disabled")).toBe("true");

    act(() => {
      deleteButton?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });

    expect(onDelete).not.toHaveBeenCalled();
    expect(bodyText()).toContain(
      "Move the remaining 4 account(s) out before deleting this group.",
    );
  });
});
