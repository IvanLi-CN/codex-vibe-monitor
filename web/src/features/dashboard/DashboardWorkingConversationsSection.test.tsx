/** @vitest-environment jsdom */

import { expect, it } from "vitest";
import {
  createConversation,
  createPreview,
  createResponse,
  host,
  renderSection,
} from "./DashboardWorkingConversationsSection.test-support";

it("shows response model as primary text and renders routing indicator on mismatch", () => {
  renderSection(
    createResponse([
      createConversation("pck-mismatch", [
        createPreview({
          id: 1,
          invokeId: "invoke-mismatch",
          occurredAt: "2026-04-04T10:05:00Z",
          status: "success",
          model: "gpt-5.5",
          requestModel: "gpt-5.4",
          responseModel: "gpt-5.5",
        }),
      ]),
    ]),
  );

  expect(host?.textContent).toContain("gpt-5.5");
  expect(
    host?.querySelector('[data-testid="dashboard-working-conversation-model-routing-indicator"]'),
  ).not.toBeNull();
});
