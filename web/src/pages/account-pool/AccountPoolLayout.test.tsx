/** @vitest-environment jsdom */
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { MemoryRouter, Route, Routes } from "react-router-dom";
import { afterEach, describe, expect, it, vi } from "vitest";
import { I18nProvider } from "../../i18n";
import AccountPoolLayout from "./AccountPoolLayout";

const viewportState = vi.hoisted(() => ({ compact: false }));

vi.mock("../../hooks/useCompactViewport", () => ({
  useCompactViewport: () => viewportState.compact,
}));

let root: Root | null = null;

function render(initialEntry: string) {
  const host = document.createElement("div");
  document.body.appendChild(host);
  root = createRoot(host);
  act(() => {
    root?.render(
      <I18nProvider initialLocale="zh" persistLocale={false}>
        <MemoryRouter initialEntries={[initialEntry]}>
          <Routes>
            <Route path="/account-pool" element={<AccountPoolLayout />}>
              <Route path="pool" element={<div data-testid="account-detail-outlet">detail</div>} />
              <Route path="groups" element={<div data-testid="groups-outlet">groups</div>} />
            </Route>
          </Routes>
        </MemoryRouter>
      </I18nProvider>,
    );
  });
  return host;
}

afterEach(() => {
  act(() => root?.unmount());
  root = null;
  document.body.replaceChildren();
  viewportState.compact = false;
});

describe("AccountPoolLayout", () => {
  it("keeps the account-pool title panel on list routes", () => {
    const host = render("/account-pool/groups");

    expect(host.querySelector("h1")?.textContent).toBe("上游");
    expect(host.querySelector("[data-testid=groups-outlet]")).not.toBeNull();
  });

  it("removes the redundant title panel on compact account detail routes", () => {
    viewportState.compact = true;
    const host = render("/account-pool/pool?upstreamAccountId=102&upstreamAccountTab=routing");

    expect(host.querySelector("h1")).toBeNull();
    expect(host.querySelector("[data-testid=account-detail-outlet]")).not.toBeNull();
  });
});
