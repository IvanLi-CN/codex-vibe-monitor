/** @vitest-environment jsdom */
import { act } from "react";
import type { ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeAll, describe, expect, it } from "vitest";
import { UpstreamAccountGroupNoteDialog } from "./UpstreamAccountGroupNoteDialog";
import type { ForwardProxyBindingNode } from "../lib/api";

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
  root = null;
  host = null;
});

function render(ui: ReactNode) {
  host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(ui);
  });
}

describe("UpstreamAccountGroupNoteDialog", () => {
  it("shows protocol badges, keeps direct available, and never renders raw subscription URLs", () => {
    const nodes: ForwardProxyBindingNode[] = [
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
        key: "vless://e8d10b05-aec8-4cee-be7d-2f5eee61b0a7@hinet-ep.707979.xyz:53842?encryption=none&security=reality&type=tcp&sni=skypapi.onedrive.com&fp=chrome&pbk=abc123&sid=long-subscription-node#Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS",
        source: "subscription",
        displayName: "Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS",
        protocolLabel: "VLESS",
        penalized: false,
        selectable: true,
        last24h: [],
      },
      {
        key: "drain-node",
        source: "manual",
        displayName: "Drain Node",
        protocolLabel: "HTTP",
        penalized: true,
        selectable: false,
        last24h: [],
      },
    ];

    render(
      <UpstreamAccountGroupNoteDialog
        open
        groupName="latam"
        note=""
        existing
        boundProxyKeys={["__direct__"]}
        availableProxyNodes={nodes}
        onNoteChange={() => undefined}
        onBoundProxyKeysChange={() => undefined}
        onClose={() => undefined}
        onSave={() => undefined}
        title="Edit group settings"
        existingDescription="Existing group"
        draftDescription="Draft group"
        noteLabel="Group note"
        notePlaceholder="Add note"
        cancelLabel="Cancel"
        saveLabel="Save"
        closeLabel="Close"
        existingBadgeLabel="Persisted group"
        draftBadgeLabel="Draft group"
        proxyBindingsLabel="Bound proxy nodes"
        proxyBindingsHint="Leave empty to keep automatic routing."
        proxyBindingsAutomaticLabel="Automatic routing"
        proxyBindingsEmptyLabel="No proxy nodes available."
        proxyBindingsMissingLabel="Missing"
        proxyBindingsUnavailableLabel="Unavailable"
        proxyBindingsChartLabel="24h request trend"
        proxyBindingsChartSuccessLabel="Success"
        proxyBindingsChartFailureLabel="Failure"
        proxyBindingsChartEmptyLabel="No 24h data"
        proxyBindingsChartTotalLabel="Total requests"
        proxyBindingsChartAriaLabel="Last 24h request volume chart"
        proxyBindingsChartInteractionHint="Hover or tap for details."
        proxyBindingsChartLocaleTag="en-US"
      />,
    );

    const text = document.body.textContent || "";
    expect(text).toContain("Direct");
    expect(text).toContain("DIRECT");
    expect(text).toContain("VLESS");
    expect(text).not.toContain("vless://");

    const scrollRegion = document.querySelector(
      '[data-testid="proxy-binding-options-scroll-region"]',
    ) as HTMLElement | null;
    expect(scrollRegion).not.toBeNull();
    expect(scrollRegion?.className).toContain("overflow-y-auto");

    const dialog = document.querySelector('[role="dialog"]') as HTMLElement | null;
    expect(dialog).not.toBeNull();
    expect(dialog?.className).not.toContain("max-w-[72rem]");
    expect(dialog?.className).toContain("sm:max-w-[44rem]");

    const truncatedTitle = document.querySelector(
      '[title="Ivan-hinet-vless-vision-01KF874741GBN6MQYD6TNMYDVS"]',
    ) as HTMLElement | null;
    expect(truncatedTitle).not.toBeNull();
    expect(truncatedTitle?.className).toContain("truncate");
  });
});
