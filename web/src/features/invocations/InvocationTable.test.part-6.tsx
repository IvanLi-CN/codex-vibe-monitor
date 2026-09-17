import { expect, it } from "vitest";
import { resolveInvocationImageIntentDisplay } from "../../lib/invocation";

it("shows the image badge only for yes and direct_image", () => {
  expect(
    resolveInvocationImageIntentDisplay({
      imageIntent: "yes",
    }),
  ).toMatchObject({
    kind: "yes",
    showsChip: true,
    badgeLabelKey: "table.imageTool.badge",
    detailLabelKey: "table.imageTool.detail.yes",
  });

  expect(
    resolveInvocationImageIntentDisplay({
      imageIntent: "direct_image",
    }),
  ).toMatchObject({
    kind: "direct_image",
    showsChip: true,
    badgeLabelKey: "table.imageTool.badge",
    detailLabelKey: "table.imageTool.detail.directImage",
  });

  expect(
    resolveInvocationImageIntentDisplay({
      imageIntent: "no",
    }),
  ).toMatchObject({
    kind: "no",
    showsChip: false,
    detailLabelKey: "table.imageTool.detail.no",
  });

  expect(
    resolveInvocationImageIntentDisplay({
      imageIntent: "unknown",
    }),
  ).toMatchObject({
    kind: "unknown",
    showsChip: false,
    detailLabelKey: "table.imageTool.detail.unknown",
  });

  expect(resolveInvocationImageIntentDisplay(undefined)).toMatchObject({
    kind: "missing",
    showsChip: false,
    detailLabelKey: null,
  });
});
