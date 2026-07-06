import type { ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { I18nProvider } from "../i18n";
import {
  InvocationPhaseBadge,
  InvocationPhaseSegments,
} from "./InvocationPhaseBadge";

function renderMarkup(element: ReactNode) {
  return renderToStaticMarkup(<I18nProvider>{element}</I18nProvider>);
}

describe("InvocationPhaseBadge", () => {
  it("renders requesting badges with dynamic pulse motion and supports icon-only compact mode", () => {
    const html = renderMarkup(
      <InvocationPhaseBadge
        phase="requesting"
        appearance="inline"
        motion="dynamic"
        showLabel={false}
      />,
    );

    expect(html).toContain('data-phase="requesting"');
    expect(html).toContain('data-phase-motion="dynamic"');
    expect(html).toContain('data-phase-label-visible="false"');
    expect(html).toContain('aria-label="请求中"');
    expect(html).toContain("animate-pulse");
    expect(html).not.toContain(">请求中<");
  });

  it("renders responding badges with dynamic spin motion when labels stay visible", () => {
    const html = renderMarkup(
      <InvocationPhaseBadge phase="responding" motion="dynamic" />,
    );

    expect(html).toContain('data-phase="responding"');
    expect(html).toContain('data-phase-motion="dynamic"');
    expect(html).toContain('data-phase-label-visible="true"');
    expect(html).toContain('data-phase-icon-name="loading"');
    expect(html).toContain("animate-spin");
    expect(html).toContain(">响应中<");
  });
});

describe("InvocationPhaseSegments", () => {
  it("keeps summary segments static by default", () => {
    const html = renderMarkup(
      <InvocationPhaseSegments
        counts={{ queued: 1, requesting: 2, responding: 3 }}
        appearance="inline"
      />,
    );

    expect((html.match(/data-testid="invocation-phase-segment"/g) ?? []).length).toBe(
      3,
    );
    expect(html).toContain('data-phase-motion="static"');
    expect(html).toContain('data-phase-icon-name="message-reply-outline"');
    expect(html).not.toContain("animate-pulse");
    expect(html).not.toContain("animate-spin");
    expect(html).not.toContain('data-phase-icon-name="loading"');
  });
});
