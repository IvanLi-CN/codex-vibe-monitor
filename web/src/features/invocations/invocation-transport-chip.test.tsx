/** @vitest-environment jsdom */

import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { I18nProvider } from "../../i18n";
import { renderInvocationTransportChip } from "./invocation-transport-chip";

function renderChip(initialLocale: "zh" | "en") {
  return renderToStaticMarkup(
    <I18nProvider initialLocale={initialLocale} persistLocale={false}>
      {renderInvocationTransportChip({ transport: "websocket" })}
    </I18nProvider>,
  );
}

describe("renderInvocationTransportChip", () => {
  it("uses the Chinese historical label as the visible chip text", () => {
    const markup = renderChip("zh");

    expect(markup).toContain('title="WebSocket（历史）"');
    expect(markup).toContain(">WebSocket（历史）</span>");
    expect(markup).not.toContain(">WS</span>");
  });

  it("uses the English historical label as the visible chip text", () => {
    const markup = renderChip("en");

    expect(markup).toContain('title="WebSocket (historical)"');
    expect(markup).toContain(">WebSocket (historical)</span>");
    expect(markup).not.toContain(">WS</span>");
  });
});
