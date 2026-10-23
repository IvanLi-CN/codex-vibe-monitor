import { expect, type Page, test } from "@playwright/test";

import {
  buildDashboardActivityResponse,
  buildSummary,
  buildTimeseries,
  buildWorkingConversationsResponse,
} from "../../src/test-fixtures/dashboardWorkingConversations";

type LayoutExpectation = {
  viewport: { width: number; height: number };
  expectedColumns: number;
};

const VIEWPORTS: LayoutExpectation[] = [
  { viewport: { width: 1440, height: 900 }, expectedColumns: 2 },
  { viewport: { width: 1600, height: 900 }, expectedColumns: 3 },
  { viewport: { width: 1660, height: 900 }, expectedColumns: 4 },
  { viewport: { width: 1873, height: 900 }, expectedColumns: 4 },
];

async function installDashboardRoutes(
  page: Page,
  options: {
    currentFirstTokenMs?: number | null;
    currentTtfbMs?: number | null;
    gpt56Current?: boolean;
  } = {},
) {
  await page.route("**/events**", async (route) => {
    const url = new URL(route.request().url());
    const topicsParam = url.searchParams.get("topics");
    if (!topicsParam) {
      await route.fulfill({ status: 200, contentType: "text/event-stream", body: ": ready\n\n" });
      return;
    }

    const topics = JSON.parse(Buffer.from(topicsParam, "base64url").toString("utf8")) as Array<{
      topic: string;
      params?: Record<string, string>;
    }>;
    const frames = topics
      .filter(
        (descriptor) =>
          descriptor.topic === "dashboard.working-conversations.current" ||
          descriptor.topic === "dashboard.activity.current",
      )
      .map((descriptor) =>
        JSON.stringify({
          type: "snapshot",
          topic: descriptor,
          topic_key: "e2e-dashboard-working-conversations",
          schema_epoch: "e2e-schema-1",
          cursor: 1,
          payload:
            descriptor.topic === "dashboard.activity.current"
              ? buildDashboardActivityResponse()
              : buildWorkingConversationsResponse(options),
        }),
      )
      .map((frame) => `data: ${frame}\n\n`)
      .join("");

    await route.fulfill({
      status: 200,
      headers: { "Cache-Control": "no-cache" },
      contentType: "text/event-stream",
      body: frames || ": ready\n\n",
    });
  });

  await page.route("**/api/stats/summary**", async (route) => {
    const url = new URL(route.request().url());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildSummary(url.searchParams.get("window") ?? "today")),
    });
  });

  await page.route("**/api/stats/timeseries**", async (route) => {
    const url = new URL(route.request().url());
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildTimeseries(url.searchParams.get("range"))),
    });
  });

  await page.route("**/api/stats/prompt-cache-conversations**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildWorkingConversationsResponse(options)),
    });
  });

  await page.route("**/api/stats/dashboard-activity**", async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "application/json",
      body: JSON.stringify(buildDashboardActivityResponse()),
    });
  });
}

test.describe("Dashboard working conversations responsive layout", () => {
  for (const { viewport, expectedColumns } of VIEWPORTS) {
    test(`keeps ${expectedColumns} columns at ${viewport.width}px`, async ({ page }) => {
      await installDashboardRoutes(page);
      await page.setViewportSize(viewport);
      await page.goto("/#/dashboard");

      await expect(page.getByTestId("dashboard-working-conversations")).toBeVisible();
      await expect(page.getByTestId("dashboard-working-conversation-card")).toHaveCount(8);

      const layout = await page.evaluate(() => {
        const cards = Array.from(
          document.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-card"]',
          ),
        );
        const grid = document.querySelector<HTMLElement>(
          '[data-testid="dashboard-working-conversations-grid"]',
        );
        const root = document.documentElement;
        if (!grid || cards.length === 0) {
          throw new Error("missing working conversations grid");
        }

        const tops: number[] = [];
        const columnsPerRow = new Map<number, number>();
        for (const card of cards) {
          const top = Math.round(card.getBoundingClientRect().top);
          const matchedTop = tops.find((candidate) => Math.abs(candidate - top) <= 4) ?? top;
          if (!tops.includes(matchedTop)) tops.push(matchedTop);
          columnsPerRow.set(matchedTop, (columnsPerRow.get(matchedTop) ?? 0) + 1);
        }

        const firstRowTop = tops.sort((left, right) => left - right)[0];
        const cardWidths = cards.map((card) => Math.round(card.getBoundingClientRect().width));

        return {
          rootOverflow: root.scrollWidth - root.clientWidth,
          firstRowCount: columnsPerRow.get(firstRowTop) ?? 0,
          rowCount: tops.length,
          minCardWidth: Math.min(...cardWidths),
          maxCardWidth: Math.max(...cardWidths),
        };
      });

      test.info().annotations.push({
        type: "dashboard-working-conversations-layout",
        description: JSON.stringify({ viewport, expectedColumns, layout }),
      });

      expect(layout.rootOverflow).toBeLessThanOrEqual(1);
      expect(layout.firstRowCount).toBe(expectedColumns);
      expect(layout.minCardWidth).toBeGreaterThan(300);
      expect(layout.maxCardWidth - layout.minCardWidth).toBeLessThanOrEqual(4);

      if (expectedColumns === 4) {
        expect(layout.rowCount).toBe(2);
      }
    });
  }

  test("keeps compact invocation rows aligned on wide desktop cards", async ({ page }) => {
    await installDashboardRoutes(page, { gpt56Current: true });
    await page.setViewportSize({ width: 1660, height: 1180 });
    await page.goto("/#/dashboard");

    const compactBadge = page.locator(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
    );
    await expect(compactBadge.first()).toContainText(/远程压缩|Compact/);

    const compactCard = page
      .getByTestId("dashboard-working-conversation-card")
      .filter({
        has: page.locator(
          '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="compact"]',
        ),
      })
      .first();
    await expect(compactCard).toBeVisible();

    const layout = await compactCard.evaluate((card) => {
      const slot = card.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
      );
      const accountLine = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-account-line"]',
      );
      const accountChip = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-account-chip"]',
      );
      const slotModel = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-slot-model"]',
      );
      const usageLine = slot?.querySelector<HTMLElement>(
        '[data-testid="dashboard-working-conversation-usage-line"]',
      );
      if (!slot || !accountLine || !accountChip || !slotModel || !usageLine) {
        throw new Error("missing account line geometry anchors");
      }

      const slotRect = slot.getBoundingClientRect();
      const lineRect = accountLine.getBoundingClientRect();
      const chipRect = accountChip.getBoundingClientRect();
      const modelRect = slotModel.getBoundingClientRect();
      const modelVisual =
        slotModel.querySelector<HTMLElement>('[data-model-context-grouped="true"]') ?? slotModel;
      const modelVisualRect = modelVisual.getBoundingClientRect();
      const usageRect = usageLine.getBoundingClientRect();
      const readingsRect = slot
        .querySelector<HTMLElement>('[data-testid="dashboard-working-conversation-slot-readings"]')
        ?.getBoundingClientRect();
      const timeRect = slot
        .querySelector<HTMLElement>('[data-testid="dashboard-working-conversation-slot-time"]')
        ?.getBoundingClientRect();
      const headerRect = slot
        .querySelector<HTMLElement>('[data-testid="dashboard-working-conversation-slot-header"]')
        ?.getBoundingClientRect();
      if (!timeRect || !headerRect || !readingsRect) {
        throw new Error("missing compact invocation header geometry anchors");
      }
      const modelReadingsOverlap =
        modelVisualRect.left < readingsRect.right &&
        modelVisualRect.right > readingsRect.left &&
        modelVisualRect.top < readingsRect.bottom &&
        modelVisualRect.bottom > readingsRect.top;
      const reasoningTextOverflow = Array.from(
        card.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-reasoning-effort"] span',
        ),
      ).some((text) => text.scrollWidth > text.clientWidth);
      const reasoningContextGap = Math.max(
        0,
        ...Array.from(
          slot.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-reasoning-effort"]',
          ),
        ).map((reasoningEffort) => {
          const modelIdentity = reasoningEffort
            .closest<HTMLElement>('[data-testid="dashboard-working-conversation-slot-model"]')
            ?.querySelector<HTMLElement>(
              '[data-testid="dashboard-working-conversation-model-name"]',
            );
          if (!modelIdentity) throw new Error("missing model geometry anchor");
          return (
            reasoningEffort.getBoundingClientRect().left -
            modelIdentity.getBoundingClientRect().right
          );
        }),
      );
      const reasoningChipHeight = Math.max(
        0,
        ...Array.from(
          slot.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-reasoning-effort"]',
          ),
        ).map((reasoningEffort) => reasoningEffort.getBoundingClientRect().height),
      );
      const groupedGeometry = Array.from(
        card.querySelectorAll<HTMLElement>('[data-model-context-grouped="true"]'),
      ).map((cluster) => {
        const modelPart = cluster.querySelector<HTMLElement>('[data-model-context-part="model"]');
        const modelIcon = cluster.querySelector<SVGElement>(
          '[data-model-context-part="model"] svg',
        );
        const fastIcon = cluster.querySelector<SVGElement>(
          '[data-testid="invocation-fast-icon"] svg',
        );
        const fastPart = cluster.querySelector<HTMLElement>('[data-model-context-part="fast"]');
        const effortText = cluster.querySelector<HTMLElement>(
          '[data-model-context-part="reasoning-effort-text"]',
        );
        const reasoningPart = cluster.querySelector<HTMLElement>(
          '[data-model-context-part="reasoning-effort"]',
        );
        const generation = cluster.querySelector<HTMLElement>("[data-model-generation-segment]");
        if (!modelPart || !reasoningPart || !effortText || !generation) {
          throw new Error("missing grouped model context geometry anchors");
        }
        const modelRect = modelPart.getBoundingClientRect();
        const reasoningRect = reasoningPart.getBoundingClientRect();
        const effortTextRect = effortText.getBoundingClientRect();
        const fastPartRect = fastPart?.getBoundingClientRect();
        const modelIconRect = modelIcon?.getBoundingClientRect();
        const modelIconContainerRect = modelIcon?.parentElement?.getBoundingClientRect();
        const generationRect = generation.getBoundingClientRect();
        const fastIconRect = fastIcon?.getBoundingClientRect();
        const fastPath = fastIcon?.querySelector("path");
        const fastPathBox = fastPath?.getBBox();
        const fastPathTransform = fastPath?.getScreenCTM();
        const fastVisibleRight =
          fastPathBox && fastPathTransform
            ? new DOMPoint(fastPathBox.x + fastPathBox.width, fastPathBox.y).matrixTransform(
                fastPathTransform,
              ).x
            : null;
        const fastVisibleLeft =
          fastPathBox && fastPathTransform
            ? new DOMPoint(fastPathBox.x, fastPathBox.y).matrixTransform(fastPathTransform).x
            : null;
        const centerDelta = (iconRect: DOMRect, containerRect: DOMRect) =>
          Math.max(
            Math.abs(
              iconRect.left + iconRect.width / 2 - (containerRect.left + containerRect.width / 2),
            ),
            Math.abs(
              iconRect.top + iconRect.height / 2 - (containerRect.top + containerRect.height / 2),
            ),
          );
        const expectedDirectParts = new Set(["model", "reasoning-effort", "fast"]);
        const hasVisiblePseudo = (element: Element, pseudo: "::before" | "::after") => {
          const pseudoStyle = getComputedStyle(element, pseudo);
          const width = Number.parseFloat(pseudoStyle.width);
          const height = Number.parseFloat(pseudoStyle.height);
          return (
            pseudoStyle.display !== "none" &&
            pseudoStyle.visibility !== "hidden" &&
            Number.parseFloat(pseudoStyle.opacity || "1") > 0 &&
            (width > 0 || height > 0)
          );
        };
        const pseudoDividerCount = [cluster, ...Array.from(cluster.children)].reduce(
          (count, element) =>
            count +
            Number(hasVisiblePseudo(element, "::before")) +
            Number(hasVisiblePseudo(element, "::after")),
          0,
        );
        const internalDividerCount = Array.from(cluster.children).reduce((count, child) => {
          const part = child.getAttribute("data-model-context-part");
          const style = getComputedStyle(child);
          const hasBorder =
            Number.parseFloat(style.borderLeftWidth) > 0 ||
            Number.parseFloat(style.borderRightWidth) > 0;
          return count + Number(!part || !expectedDirectParts.has(part) || hasBorder);
        }, 0);
        return {
          modelWidth: modelRect.width,
          modelToEffortGap: reasoningRect.left - modelRect.right,
          effortToFastGap: fastPartRect ? fastPartRect.left - effortTextRect.right : null,
          effortToFastVisibleGap:
            fastVisibleLeft == null ? null : fastVisibleLeft - effortTextRect.right,
          fastVisibleWithinPart:
            fastVisibleLeft != null &&
            fastVisibleRight != null &&
            fastPartRect != null &&
            fastVisibleLeft >= fastPartRect.left - 0.5 &&
            fastVisibleRight <= fastPartRect.right + 0.5,
          generationText: generation.textContent,
          modelIconCenterDelta:
            modelIconRect && modelIconContainerRect
              ? centerDelta(modelIconRect, modelIconContainerRect)
              : null,
          generationCenterDelta:
            modelIconContainerRect && generationRect
              ? Math.abs(
                  generationRect.top +
                    generationRect.height / 2 -
                    (modelIconContainerRect.top + modelIconContainerRect.height / 2),
                )
              : null,
          modelStartInset: modelRect.left - cluster.getBoundingClientRect().left,
          modelEndInset: cluster.getBoundingClientRect().right - modelRect.right,
          fastVisibleEndInset: fastVisibleRight
            ? cluster.getBoundingClientRect().right - fastVisibleRight
            : null,
          fastIconCenterDelta:
            fastIconRect && fastPartRect ? centerDelta(fastIconRect, fastPartRect) : null,
          effortTextOverflow: effortText.scrollWidth > effortText.clientWidth,
          markerCount: cluster.querySelectorAll(
            '[data-model-context-part="reasoning-effort-marker"]',
          ).length,
          internalDividerCount: internalDividerCount + pseudoDividerCount,
        };
      });

      return {
        chipModelTopDelta: Math.abs(chipRect.top - accountLine.getBoundingClientRect().top),
        modelWithinSlot:
          modelRect.left >= slotRect.left - 1 && modelRect.right <= slotRect.right + 1,
        modelTimeTopDelta: Math.abs(modelVisualRect.top - timeRect.top),
        readingsTimeTopDelta: Math.abs(readingsRect.top - timeRect.top),
        readingsRightDelta: Math.abs(readingsRect.right - headerRect.right),
        modelWithinHeader: modelRect.bottom <= headerRect.bottom + 1,
        modelReadingsOverlap,
        usageRightDelta: Math.abs(usageRect.right - accountLine.getBoundingClientRect().right),
        lineOverflowRight: lineRect.right - slotRect.right,
        lineHeight: lineRect.height,
        reasoningTextOverflow,
        reasoningContextGap,
        reasoningChipHeight,
        groupedGeometry,
      };
    });

    expect(layout.chipModelTopDelta).toBeLessThanOrEqual(4);
    expect(layout.modelWithinSlot).toBe(true);
    expect(layout.modelTimeTopDelta).toBeLessThanOrEqual(3);
    expect(layout.readingsTimeTopDelta).toBeLessThanOrEqual(3);
    expect(layout.readingsRightDelta).toBeLessThanOrEqual(1);
    expect(layout.modelWithinHeader).toBe(true);
    expect(layout.modelReadingsOverlap).toBe(false);
    expect(layout.usageRightDelta).toBeLessThanOrEqual(1);
    expect(layout.lineOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.lineHeight).toBeLessThan(32);
    expect(layout.reasoningTextOverflow).toBe(false);
    expect(layout.reasoningContextGap).toBeLessThanOrEqual(8);
    expect(layout.reasoningChipHeight).toBeLessThanOrEqual(20);
    expect(layout.groupedGeometry.length).toBeGreaterThan(0);
    expect(layout.groupedGeometry.every((geometry) => geometry.modelWidth >= 20)).toBe(true);
    expect(layout.groupedGeometry.every((geometry) => geometry.generationText)).toBe(true);
    expect(layout.groupedGeometry.every((geometry) => !geometry.effortTextOverflow)).toBe(true);
    expect(
      layout.groupedGeometry.every(
        (geometry) =>
          (geometry.modelIconCenterDelta ?? Number.POSITIVE_INFINITY) <= 0.5 &&
          (geometry.generationCenterDelta ?? Number.POSITIVE_INFINITY) <= 1,
      ),
    ).toBe(true);
    expect(
      layout.groupedGeometry.every(
        (geometry) =>
          geometry.modelStartInset >= 4 &&
          geometry.modelEndInset >= 4 &&
          (geometry.fastVisibleEndInset ?? Number.POSITIVE_INFINITY) >= 4 &&
          Math.abs(
            geometry.modelStartInset - (geometry.fastVisibleEndInset ?? Number.NEGATIVE_INFINITY),
          ) <= 0.5,
      ),
    ).toBe(true);
    expect(
      layout.groupedGeometry.every(
        (geometry) => geometry.modelToEffortGap >= 3 && geometry.modelToEffortGap <= 5,
      ),
    ).toBe(true);
    expect(
      layout.groupedGeometry.every(
        (geometry) =>
          (geometry.effortToFastGap ?? Number.POSITIVE_INFINITY) >= 3 &&
          (geometry.effortToFastGap ?? Number.NEGATIVE_INFINITY) <= 5,
      ),
    ).toBe(true);
    expect(
      layout.groupedGeometry.every(
        (geometry) =>
          (geometry.effortToFastVisibleGap ?? Number.POSITIVE_INFINITY) >= 2.5 &&
          (geometry.effortToFastVisibleGap ?? Number.NEGATIVE_INFINITY) <= 4.5,
      ),
    ).toBe(true);
    expect(layout.groupedGeometry.every((geometry) => geometry.markerCount === 0)).toBe(true);
    expect(layout.groupedGeometry.every((geometry) => geometry.internalDividerCount === 0)).toBe(
      true,
    );
  });

  test("keeps three slots and records inside a narrow viewport", async ({ page }) => {
    await installDashboardRoutes(page, {
      gpt56Current: true,
      mismatchCurrent: true,
    });
    await page.setViewportSize({ width: 393, height: 852 });
    await page.goto("/#/dashboard");

    await expect(page.getByTestId("dashboard-working-conversations")).toBeVisible();
    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const cards = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        ),
      );
      const slots = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-slot"]',
        ),
      );
      const placeholders = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-placeholder"]',
        ),
      );
      const labels = document.querySelectorAll(
        '[data-testid="dashboard-working-conversation-slot-label"]',
      );
      const usageLines = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-usage-line"]',
        ),
      );
      const accountLines = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-account-line"]',
        ),
      );
      const reasoningTextOverflow = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-working-conversation-reasoning-effort"] span',
        ),
      ).some((text) => text.scrollWidth > text.clientWidth);
      const reasoningChipHeight = Math.max(
        0,
        ...Array.from(
          document.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-reasoning-effort"]',
          ),
        ).map((reasoningEffort) => reasoningEffort.getBoundingClientRect().height),
      );
      const groupedContexts = Array.from(
        document.querySelectorAll<HTMLElement>('[data-model-context-grouped="true"]'),
      ).map((cluster) => {
        const modelPart = cluster.querySelector<HTMLElement>('[data-model-context-part="model"]');
        const modelIcon = cluster.querySelector<SVGElement>(
          '[data-model-context-part="model"] svg',
        );
        const fastIcon = cluster.querySelector<SVGElement>(
          '[data-testid="invocation-fast-icon"] svg',
        );
        const fastPart = cluster.querySelector<HTMLElement>('[data-model-context-part="fast"]');
        const effortText = cluster.querySelector<HTMLElement>(
          '[data-model-context-part="reasoning-effort-text"]',
        );
        const reasoningPart = cluster.querySelector<HTMLElement>(
          '[data-model-context-part="reasoning-effort"]',
        );
        const generation = cluster.querySelector<HTMLElement>("[data-model-generation-segment]");
        if (!modelPart || !reasoningPart || !effortText || !fastPart || !generation) {
          throw new Error("missing mobile grouped model context geometry anchors");
        }
        const modelRect = modelPart.getBoundingClientRect();
        const modelIdentities = Array.from(
          modelPart.querySelectorAll<HTMLElement>("[data-model-context-identity]"),
        );
        const modelIdentityRectsWithinModel = modelIdentities.every((identity) => {
          const rect = identity.getBoundingClientRect();
          return (
            rect.width > 0 &&
            rect.height > 0 &&
            rect.left >= modelRect.left - 0.5 &&
            rect.right <= modelRect.right + 0.5 &&
            rect.top >= modelRect.top - 0.5 &&
            rect.bottom <= modelRect.bottom + 0.5
          );
        });
        const modelIdentityGenerationCount = modelIdentities.filter((identity) =>
          identity.querySelector("[data-model-generation-segment]"),
        ).length;
        const reasoningRect = reasoningPart.getBoundingClientRect();
        const effortTextRect = effortText.getBoundingClientRect();
        const fastPartRect = fastPart.getBoundingClientRect();
        const clusterRect = cluster.getBoundingClientRect();
        const modelIconRect = modelIcon?.getBoundingClientRect();
        const modelIconContainerRect = modelIcon?.parentElement?.getBoundingClientRect();
        const fastIconRect = fastIcon?.getBoundingClientRect();
        const fastPath = fastIcon?.querySelector("path");
        const fastPathBox = fastPath?.getBBox();
        const fastPathTransform = fastPath?.getScreenCTM();
        const fastVisibleLeft =
          fastPathBox && fastPathTransform
            ? new DOMPoint(fastPathBox.x, fastPathBox.y).matrixTransform(fastPathTransform).x
            : null;
        const fastVisibleRight =
          fastPathBox && fastPathTransform
            ? new DOMPoint(fastPathBox.x + fastPathBox.width, fastPathBox.y).matrixTransform(
                fastPathTransform,
              ).x
            : null;
        const centerDelta = (iconRect: DOMRect, containerRect: DOMRect) =>
          Math.max(
            Math.abs(
              iconRect.left + iconRect.width / 2 - (containerRect.left + containerRect.width / 2),
            ),
            Math.abs(
              iconRect.top + iconRect.height / 2 - (containerRect.top + containerRect.height / 2),
            ),
          );
        const expectedDirectParts = new Set(["model", "reasoning-effort", "fast"]);
        const hasVisiblePseudo = (element: Element, pseudo: "::before" | "::after") => {
          const pseudoStyle = getComputedStyle(element, pseudo);
          const width = Number.parseFloat(pseudoStyle.width);
          const height = Number.parseFloat(pseudoStyle.height);
          return (
            pseudoStyle.display !== "none" &&
            pseudoStyle.visibility !== "hidden" &&
            Number.parseFloat(pseudoStyle.opacity || "1") > 0 &&
            (width > 0 || height > 0)
          );
        };
        const pseudoDividerCount = [cluster, ...Array.from(cluster.children)].reduce(
          (count, element) =>
            count +
            Number(hasVisiblePseudo(element, "::before")) +
            Number(hasVisiblePseudo(element, "::after")),
          0,
        );
        const internalDividerCount = Array.from(cluster.children).reduce((count, child) => {
          const part = child.getAttribute("data-model-context-part");
          const style = getComputedStyle(child);
          const hasBorder =
            Number.parseFloat(style.borderLeftWidth) > 0 ||
            Number.parseFloat(style.borderRightWidth) > 0;
          return count + Number(!part || !expectedDirectParts.has(part) || hasBorder);
        }, 0);
        const partsWithinCluster = [modelRect, reasoningRect, effortTextRect, fastPartRect].every(
          (partRect) =>
            partRect.left >= clusterRect.left - 1 &&
            partRect.right <= clusterRect.right + 1 &&
            partRect.top >= clusterRect.top - 1 &&
            partRect.bottom <= clusterRect.bottom + 1,
        );
        return {
          modelWidth: modelRect.width,
          modelIdentityCount: modelIdentities.length,
          modelIdentityGenerationCount,
          modelIdentityRectsWithinModel,
          modelToEffortGap: reasoningRect.left - modelRect.right,
          effortToFastGap: fastPartRect.left - effortTextRect.right,
          effortToFastVisibleGap:
            fastVisibleLeft == null ? null : fastVisibleLeft - effortTextRect.right,
          fastVisibleWithinPart:
            fastVisibleLeft != null &&
            fastVisibleRight != null &&
            fastVisibleLeft >= fastPartRect.left - 0.5 &&
            fastVisibleRight <= fastPartRect.right + 0.5,
          generationText: generation.textContent,
          modelIconCenterDelta:
            modelIconRect && modelIconContainerRect
              ? centerDelta(modelIconRect, modelIconContainerRect)
              : null,
          fastIconCenterDelta: fastIconRect ? centerDelta(fastIconRect, fastPartRect) : null,
          effortTextOverflow: effortText.scrollWidth > effortText.clientWidth,
          partsWithinCluster,
          markerCount: cluster.querySelectorAll(
            '[data-model-context-part="reasoning-effort-marker"]',
          ).length,
          internalDividerCount: internalDividerCount + pseudoDividerCount,
          effortText: effortText.textContent,
        };
      });
      const compactSlotHeaderRows = slots
        .filter((slot) => !slot.querySelector('[data-model-context-identity="request"]'))
        .map((slot) => {
          const time = slot.querySelector<HTMLElement>(
            '[data-testid="dashboard-working-conversation-slot-time"]',
          );
          const model = slot.querySelector<HTMLElement>(
            '[data-testid="dashboard-working-conversation-slot-model"]',
          );
          if (!time || !model) throw new Error("missing mobile compact header geometry anchors");
          const modelVisual =
            model.querySelector<HTMLElement>('[data-model-context-grouped="true"]') ?? model;
          const timeRect = time.getBoundingClientRect();
          const modelRect = modelVisual.getBoundingClientRect();
          const readings = slot.querySelector<HTMLElement>(
            '[data-testid="dashboard-working-conversation-slot-readings"]',
          );
          if (!readings) throw new Error("missing mobile compact readings geometry anchor");
          return {
            modelTopDelta: Math.abs(timeRect.top - modelRect.top),
            readingsTopDelta: Math.abs(timeRect.top - readings.getBoundingClientRect().top),
          };
        });
      const compactSlotHeaderOverlaps = slots
        .filter((slot) => !slot.querySelector('[data-model-context-identity="request"]'))
        .map((slot) => {
          const model = slot.querySelector<HTMLElement>(
            '[data-testid="dashboard-working-conversation-slot-model"]',
          );
          const readings = slot.querySelector<HTMLElement>(
            '[data-testid="dashboard-working-conversation-slot-readings"]',
          );
          if (!model || !readings) throw new Error("missing mobile compact overlap anchors");
          const modelVisual =
            model.querySelector<HTMLElement>('[data-model-context-grouped="true"]') ?? model;
          const modelRect = modelVisual.getBoundingClientRect();
          const readingsRect = readings.getBoundingClientRect();
          return (
            modelRect.left < readingsRect.right &&
            modelRect.right > readingsRect.left &&
            modelRect.top < readingsRect.bottom &&
            modelRect.bottom > readingsRect.top
          );
        });
      return {
        rootOverflow: root.scrollWidth - root.clientWidth,
        cards: cards.length,
        slotCount: slots.length + placeholders.length,
        labels: labels.length,
        usageRightAligned: usageLines.every((line) => {
          const accountLine = line.closest<HTMLElement>(
            '[data-testid="dashboard-working-conversation-account-line"]',
          );
          return (
            accountLine != null &&
            Math.abs(
              line.getBoundingClientRect().right - accountLine.getBoundingClientRect().right,
            ) <= 1
          );
        }),
        accountLineOverflow: Math.max(
          0,
          ...accountLines.map(
            (line) =>
              line.getBoundingClientRect().right -
              line.parentElement!.getBoundingClientRect().right,
          ),
        ),
        reasoningTextOverflow,
        reasoningChipHeight,
        groupedContexts,
        compactSlotHeaderRows,
        compactSlotHeaderOverlaps,
      };
    });

    expect(layout.rootOverflow).toBeLessThanOrEqual(1);
    expect(layout.cards).toBeGreaterThan(0);
    expect(layout.slotCount).toBeGreaterThanOrEqual(layout.cards * 3);
    expect(layout.labels).toBe(0);
    expect(layout.usageRightAligned).toBe(true);
    expect(layout.accountLineOverflow).toBeLessThanOrEqual(1);
    expect(layout.reasoningTextOverflow).toBe(false);
    expect(layout.reasoningChipHeight).toBeLessThanOrEqual(20);
    expect(layout.compactSlotHeaderRows.length).toBeGreaterThan(0);
    expect(layout.compactSlotHeaderRows.every((row) => row.modelTopDelta <= 3)).toBe(true);
    expect(layout.compactSlotHeaderRows.every((row) => row.readingsTopDelta <= 3)).toBe(true);
    expect(layout.compactSlotHeaderOverlaps.every((overlap) => !overlap)).toBe(true);
    expect(layout.groupedContexts.length).toBeGreaterThan(0);
    expect(layout.groupedContexts.every((context) => context.markerCount === 0)).toBe(true);
    expect(layout.groupedContexts.every((context) => context.effortText === "max")).toBe(true);
    expect(layout.groupedContexts.every((context) => context.modelWidth >= 20)).toBe(true);
    expect(layout.groupedContexts.every((context) => context.generationText)).toBe(true);
    const mismatchContexts = layout.groupedContexts.filter(
      (context) => context.modelIdentityCount === 2,
    );
    expect(mismatchContexts.length).toBeGreaterThan(0);
    expect(mismatchContexts.every((context) => context.modelIdentityGenerationCount === 2)).toBe(
      true,
    );
    expect(mismatchContexts.every((context) => context.modelIdentityRectsWithinModel)).toBe(true);
    expect(
      layout.groupedContexts.every(
        (context) => context.modelToEffortGap >= 3 && context.modelToEffortGap <= 5,
      ),
    ).toBe(true);
    expect(layout.groupedContexts.every((context) => !context.effortTextOverflow)).toBe(true);
    expect(layout.groupedContexts.every((context) => context.partsWithinCluster)).toBe(true);
    expect(
      layout.groupedContexts.every(
        (context) => (context.modelIconCenterDelta ?? Number.POSITIVE_INFINITY) <= 0.5,
      ),
    ).toBe(true);
    expect(
      layout.groupedContexts.every(
        (context) => context.effortToFastGap >= 3 && context.effortToFastGap <= 5,
      ),
    ).toBe(true);
    expect(
      layout.groupedContexts.every(
        (context) =>
          (context.effortToFastVisibleGap ?? Number.POSITIVE_INFINITY) >= 2.5 &&
          (context.effortToFastVisibleGap ?? Number.NEGATIVE_INFINITY) <= 4.5,
      ),
    ).toBe(true);
    expect(layout.groupedContexts.every((context) => context.fastVisibleWithinPart)).toBe(true);
    expect(layout.groupedContexts.every((context) => context.internalDividerCount === 0)).toBe(
      true,
    );
  });

  test("keeps static missing-history slots at the shared baseline on desktop and mobile", async ({
    page,
  }) => {
    await installDashboardRoutes(page);

    for (const viewport of [
      { width: 1660, height: 900 },
      { width: 393, height: 852 },
    ]) {
      await page.setViewportSize(viewport);
      await page.goto("/#/dashboard");
      await expect(page.getByTestId("dashboard-working-conversations")).toBeVisible();

      const layout = await page.evaluate(() => {
        const currentSlot = document.querySelector<HTMLElement>(
          '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"][aria-label*="wc-3-a"]',
        );
        const card = currentSlot?.closest<HTMLElement>(
          '[data-testid="dashboard-working-conversation-card"]',
        );
        if (!currentSlot || !card) throw new Error("missing current-only geometry anchors");

        const placeholders = Array.from(
          card.querySelectorAll<HTMLElement>(
            '[data-testid="dashboard-working-conversation-placeholder"]',
          ),
        );
        const latencyPills = currentSlot.querySelector<HTMLElement>(
          '[data-testid="dashboard-compact-latency-pills"]',
        );
        if (!latencyPills) throw new Error("missing compact latency pills");

        return {
          rootOverflow: document.documentElement.scrollWidth - document.documentElement.clientWidth,
          normalHeight: currentSlot.getBoundingClientRect().height,
          placeholderHeights: placeholders.map(
            (placeholder) => placeholder.getBoundingClientRect().height,
          ),
          labels: placeholders.map((placeholder) => placeholder.textContent?.trim() ?? ""),
          skeletonLines: card.querySelectorAll(".working-conversation-placeholder-line").length,
          ariaLiveCount: card.querySelectorAll(
            '[data-testid="dashboard-working-conversation-placeholder"][aria-live]',
          ).length,
          latencyGap: window.getComputedStyle(latencyPills).columnGap,
          latencyValues: Array.from(
            latencyPills.querySelectorAll<HTMLElement>(
              '[data-testid="dashboard-compact-latency-ttft"], [data-testid="dashboard-compact-latency-response"]',
            ),
          ).map((element) => element.textContent ?? ""),
        };
      });

      expect(layout.rootOverflow).toBeLessThanOrEqual(1);
      expect(layout.placeholderHeights).toHaveLength(2);
      expect(Math.max(...layout.placeholderHeights)).toBeLessThanOrEqual(58);
      expect(Math.min(...layout.placeholderHeights)).toBeGreaterThanOrEqual(56);
      expect(layout.normalHeight).toBeGreaterThanOrEqual(Math.max(...layout.placeholderHeights));
      expect(layout.labels).toEqual(expect.arrayContaining(["暂无上一条调用", "暂无更早调用"]));
      expect(layout.skeletonLines).toBe(0);
      expect(layout.ariaLiveCount).toBe(0);
      expect(layout.latencyGap).toBe("4px");
      expect(layout.latencyValues.some((value) => /\d\s+s/.test(value))).toBe(false);
    }
  });

  test("keeps long recent error summaries inside their upstream account cards", async ({
    page,
  }) => {
    await installDashboardRoutes(page);
    await page.setViewportSize({ width: 1660, height: 1180 });
    await page.goto("/#/dashboard");

    await page.getByRole("tab", { name: "上游账号" }).click();
    await expect(page.getByTestId("dashboard-upstream-account-card")).toHaveCount(2);

    const layout = await page.evaluate(() => {
      const root = document.documentElement;
      const grid = document.querySelector<HTMLElement>(
        '[data-testid="dashboard-upstream-account-grid"]',
      );
      const cards = Array.from(
        document.querySelectorAll<HTMLElement>('[data-testid="dashboard-upstream-account-card"]'),
      );
      const recentRows = Array.from(
        document.querySelectorAll<HTMLElement>(
          '[data-testid="dashboard-upstream-account-recent-row"]',
        ),
      );
      if (!grid || cards.length !== 2 || recentRows.length === 0) {
        throw new Error("missing upstream account overflow geometry anchors");
      }

      const gridRect = grid.getBoundingClientRect();
      const cardOverflowRight = Math.max(
        ...cards.map((card) => card.getBoundingClientRect().right - gridRect.right),
      );
      const rowOverflowRight = Math.max(
        ...recentRows.map((row) => {
          const card = row.closest<HTMLElement>('[data-testid="dashboard-upstream-account-card"]');
          if (!card) throw new Error("missing recent row card parent");
          return row.getBoundingClientRect().right - card.getBoundingClientRect().right;
        }),
      );

      return {
        rootOverflow: root.scrollWidth - root.clientWidth,
        gridOverflowRight: gridRect.right - root.clientWidth,
        cardOverflowRight,
        rowOverflowRight,
      };
    });

    expect(layout.rootOverflow).toBeLessThanOrEqual(1);
    expect(layout.gridOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.cardOverflowRight).toBeLessThanOrEqual(1);
    expect(layout.rowOverflowRight).toBeLessThanOrEqual(1);
  });
});
