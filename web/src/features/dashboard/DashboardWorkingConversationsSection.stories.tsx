import { expect, userEvent, waitFor, within } from "storybook/test";
import { DashboardWorkingConversationsSection } from "./DashboardWorkingConversationsSection";
import {
  accountPlanBadgeResponse,
  assertImageEditEndpointRecentRow,
  assertUpstreamAccountTabStory,
  assignedAccountFailureSemanticsResponse,
  buildCards,
  buildDashboardHistoryEvidenceFixtures,
  createConversation,
  createImageEditEndpointUpstreamAccountActivityStoryResponse,
  createPoolRoutingAccountStatesResponse,
  createPreview,
  createRelativeStoryIso,
  createRequestingOnlyResponse,
  createResponse,
  createRunningOnlyResponse,
  createUpstreamAccountActivityStoryResponse,
  createUpstreamAccountRecentLayoutStoryResponse,
  createWarningSuccessUpstreamAccountActivityResponse,
  currentAndPreviousResponse,
  currentOnlyResponse,
  DrawerPreviewStory,
  ForcedWorkspaceViewStory,
  failedClickableResponse,
  failedStatusDedupResponse,
  gpt56ModelContextResponse,
  gpt56ModelContextWithoutReasoningResponse,
  imageEndpointChipResponse,
  interruptedRecoveryResponse,
  requireFixture,
  type Story,
  meta as storyMeta,
  summaryThresholdResponse,
  summaryThresholdUpstreamActivity,
  transportBadgeResponse,
  warningSuccessConversationResponse,
} from "./DashboardWorkingConversationsSection.stories.support";

const meta = { ...storyMeta };
export default meta;

export const CurrentAndPrevious: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: buildCards(currentAndPreviousResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    const ttftLatency = currentSlot.querySelector('[data-testid="dashboard-compact-latency-ttft"]');
    const responseLatency = currentSlot.querySelector(
      '[data-testid="dashboard-compact-latency-response"]',
    );
    const reasoningEffort = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-reasoning-effort"]',
    );
    const reasoningText = reasoningEffort?.querySelector("span");
    const modelIdentity = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-model-name"]',
    );
    if (
      !(ttftLatency instanceof HTMLElement) ||
      !(responseLatency instanceof HTMLElement) ||
      !(reasoningEffort instanceof HTMLElement) ||
      !(reasoningText instanceof HTMLElement) ||
      !(modelIdentity instanceof HTMLElement)
    ) {
      throw new Error("missing compact latency readings");
    }
    const slotHeader = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    if (!(slotHeader instanceof HTMLElement)) {
      throw new Error("missing slot header");
    }
    await expect(
      slotHeader.querySelector('[data-testid="dashboard-working-conversation-slot-label"]'),
    ).toBeNull();
    await expect(
      slotHeader.querySelector('[data-testid="dashboard-working-conversation-slot-model"]'),
    ).toBeInTheDocument();
    await expect(
      canvasElement.querySelectorAll('[data-testid="dashboard-working-conversation-slot"]'),
    ).toHaveLength(3);
    await expect(slotHeader).toContainElement(ttftLatency);
    await expect(slotHeader).toContainElement(responseLatency);
    await expect(ttftLatency).toHaveTextContent("0.7s");
    await expect(responseLatency).toHaveTextContent("0.3s");
    await expect(ttftLatency.parentElement).toHaveClass("gap-1");
    await expect(ttftLatency.parentElement).not.toHaveTextContent(/\d\s+s/);
    await expect(reasoningEffort).toHaveTextContent("medium");
    await expect(reasoningText.scrollWidth).toBeLessThanOrEqual(reasoningText.clientWidth);
    await expect(reasoningEffort.getBoundingClientRect().height).toBeLessThanOrEqual(17);
    await expect(
      reasoningEffort.getBoundingClientRect().left - modelIdentity.getBoundingClientRect().right,
    ).toBeLessThanOrEqual(8);
    await expect(ttftLatency.className).not.toMatch(/rounded|border|bg-/);
    await expect(responseLatency.className).not.toMatch(/rounded|border|bg-/);
    const imageBadge = currentSlot.querySelector('[data-testid="dashboard-image-tool-icon-badge"]');
    if (!(imageBadge instanceof HTMLElement)) {
      throw new Error("missing image tool icon badge");
    }
    await expect(imageBadge).toHaveAttribute(
      "aria-label",
      expect.stringMatching(/图片工具|Image tool/),
    );
    await expect(imageBadge.className).toMatch(/rounded-full/);
    await expect(imageBadge.className).toMatch(/border/);
    const usageLine = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-line"]',
    );
    const usageHit = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const usageCost = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    if (
      !(usageLine instanceof HTMLElement) ||
      !(usageHit instanceof HTMLElement) ||
      !(usageCost instanceof HTMLElement)
    ) {
      throw new Error("missing compact usage line");
    }
    await expect(usageLine).toHaveTextContent(/Hit .*Token .*\$/);
    await expect(usageLine.textContent ?? "").not.toMatch(/\bIN\b|\bCW\b|\bO\b/);
    await expect(usageHit).toHaveAttribute("data-summary-tone", "warning");
    await expect(usageCost).toHaveAttribute("data-summary-tone", "warning");
    await expect(currentSlot).not.toHaveTextContent(/RQ |UP |ED |TT /);
  },
};

export const GPT56ModelContextCluster: Story = {
  tags: ["test"],
  globals: { viewport: { value: "desktop1660", isRotated: false } },
  parameters: {
    docs: {
      description: {
        story:
          "GPT-5.6 invocation metadata keeps one error-tone max marker, reasoning effort, and FAST state in a compact reusable visual cluster.",
      },
    },
  },
  args: {
    activeRange: "today",
    cards: buildCards(gpt56ModelContextResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const cluster = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-model-context"]',
    );
    if (!(cluster instanceof HTMLElement)) {
      throw new Error("missing GPT-5.6 model context cluster");
    }

    await expect(cluster).toHaveAttribute("data-model-context-grouped", "true");
    await expect(cluster).toHaveAttribute("aria-label", expect.stringContaining("gpt-5.6-sol"));
    await expect(cluster.querySelector('[data-model-icon="white-balance-sunny"]')).not.toBeNull();
    await expect(
      cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]'),
    ).toHaveClass("bg-error/85");
    await expect(
      cluster.querySelector(
        '[data-testid="dashboard-working-conversation-model-context-reasoning-effort"]',
      ),
    ).toHaveTextContent("max");
    await expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
  },
};

export const GPT56ModelContextMobile393: Story = {
  tags: ["test"],
  globals: { viewport: { value: "mobile393", isRotated: false } },
  parameters: {
    docs: {
      description: {
        story:
          "Mobile 393px renders the same GPT-5.6 max reasoning marker, effort text, and FAST state as the desktop context cluster.",
      },
    },
  },
  args: {
    activeRange: "today",
    cards: buildCards(gpt56ModelContextResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const cluster = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-model-context"]',
    );
    if (!(cluster instanceof HTMLElement)) {
      throw new Error("missing GPT-5.6 model context cluster");
    }

    await expect(cluster).toHaveAttribute("data-model-context-grouped", "true");
    await expect(cluster.querySelector('[data-model-icon="white-balance-sunny"]')).not.toBeNull();
    await expect(
      cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]'),
    ).toHaveClass("bg-error/85");
    await expect(
      cluster.querySelector(
        '[data-testid="dashboard-working-conversation-model-context-reasoning-effort"]',
      ),
    ).toHaveTextContent("max");
    await expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
  },
};

export const GPT56ModelContextWithoutReasoning: Story = {
  tags: ["test"],
  globals: { viewport: { value: "desktop1660", isRotated: false } },
  parameters: {
    docs: {
      description: {
        story:
          "When reasoning data is unavailable, the grouped context omits only the marker and effort text while retaining the model identity and FAST accessibility.",
      },
    },
  },
  args: {
    activeRange: "today",
    cards: buildCards(gpt56ModelContextWithoutReasoningResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const cluster = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-model-context"]',
    );
    if (!(cluster instanceof HTMLElement)) {
      throw new Error("missing GPT-5.6 model context cluster");
    }

    await expect(cluster).toHaveAttribute("data-model-context-grouped", "true");
    await expect(cluster.querySelector('[data-model-icon="white-balance-sunny"]')).not.toBeNull();
    await expect(cluster.querySelector('[data-model-context-part="reasoning-effort"]')).toBeNull();
    await expect(
      cluster.querySelector('[data-model-context-part="reasoning-effort-marker"]'),
    ).toBeNull();
    await expect(cluster).not.toHaveTextContent("—");
    await expect(cluster.querySelector('[data-testid="invocation-fast-icon"]')).not.toBeNull();
    await expect(cluster).toHaveAttribute("aria-label", expect.stringContaining("gpt-5.6-sol"));
  },
};

export const CurrentOnlyPlaceholder: Story = {
  tags: ["test"],
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Desktop state with one real invocation and two static missing-history slots sharing the 57px baseline.",
      },
    },
  },
  args: {
    activeRange: "today",
    cards: buildCards(currentOnlyResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const placeholders = canvasElement.querySelectorAll(
      '[data-testid="dashboard-working-conversation-placeholder"]',
    );
    await expect(placeholders).toHaveLength(2);
    await expect(
      canvasElement.querySelector(
        '[data-testid="dashboard-working-conversation-placeholder"][data-slot-kind="previous"]',
      ),
    ).toHaveTextContent(/暂无上一条调用|No previous invocation yet/);
    await expect(
      canvasElement.querySelector(
        '[data-testid="dashboard-working-conversation-placeholder"][data-slot-kind="earlier"]',
      ),
    ).toHaveTextContent(/暂无更早调用|No earlier invocation yet/);
    for (const placeholder of placeholders) {
      await expect(placeholder).toHaveAttribute("role", "group");
      await expect(placeholder).not.toHaveAttribute("aria-live");
      await expect(
        placeholder.querySelectorAll(".working-conversation-placeholder-line"),
      ).toHaveLength(0);
      await expect(
        placeholder.querySelector(
          '[data-testid="dashboard-working-conversation-placeholder-label"]',
        ),
      ).not.toBeNull();
    }
  },
};

export const CurrentOnlyPlaceholderMobile393: Story = {
  ...CurrentOnlyPlaceholder,
  parameters: {
    viewport: { defaultViewport: "mobile393" },
    docs: {
      description: {
        story:
          "393x852 responsive state keeps the static missing-history labels and the 57px slot baseline readable without skeleton or loading treatment.",
      },
    },
  },
};

export const ImageEndpointChips: Story = {
  parameters: {
    docs: {
      description: {
        story:
          "Shared endpoint chip coverage for direct image endpoints inside the dashboard working-conversation slots. Image-family paths render as `image/gen` or `image`, and the dashboard-specific image icon badge stays hidden to avoid duplicate signals.",
      },
    },
  },
  args: {
    activeRange: "today",
    cards: buildCards(imageEndpointChipResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }

    const imageEndpointBadge = currentSlot.querySelector(
      '[data-testid="invocation-endpoint-badge"][data-endpoint-kind="image_gen"]',
    );
    if (!(imageEndpointBadge instanceof HTMLElement)) {
      throw new Error("missing image endpoint badge");
    }

    await expect(imageEndpointBadge).toHaveTextContent("image/gen");
    if (currentSlot.querySelector('[data-testid="dashboard-image-tool-icon-badge"]') != null) {
      throw new Error("unexpected image tool icon badge");
    }
    await expect(currentSlot).not.toHaveTextContent("/v1/images/generations");
  },
};

export const WarningSuccessConversationCard: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(warningSuccessConversationResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Conversation workspace card showing the dedicated warning-success status for future pure_downstream_closed rows while keeping the rest of the dashboard layout unchanged.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const statusNode = canvasElement.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    );
    if (!(statusNode instanceof HTMLElement)) {
      throw new Error("missing warning success status node");
    }
    await expect(statusNode.getAttribute("title")).toBeNull();
    await expect(statusNode.getAttribute("aria-label") ?? "").toContain("警告成功");
    await userEvent.hover(statusNode);
    await expect(within(document.body).getByRole("tooltip")).toHaveTextContent("警告成功");
    await expect(canvasElement.textContent ?? "").toContain("Pool Alpha");
  },
};

export const ManualBindingBadges: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation(
          "pck-story-manual-binding-group",
          [
            createPreview({
              id: 8101,
              invokeId: "story-manual-binding-group",
              occurredAt: "2026-04-04T10:04:55Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "pool-alpha@example.com",
            }),
            createPreview({
              id: 8100,
              invokeId: "story-manual-binding-group-prev",
              occurredAt: createRelativeStoryIso(-(2 * 60_000 + 5_000)),
              status: "completed",
            }),
          ],
          {
            manualBinding: {
              bindingKind: "group",
              groupName: "CIII",
              upstreamAccountId: null,
              upstreamAccountName: null,
            },
          },
        ),
        createConversation(
          "pck-story-manual-binding-account",
          [
            createPreview({
              id: 8201,
              invokeId: "story-manual-binding-account",
              occurredAt: createRelativeStoryIso(-35_000),
              status: "completed",
              upstreamAccountId: 108,
              upstreamAccountName: "Codex Pro - Tokyo",
            }),
          ],
          {
            manualBinding: {
              bindingKind: "upstreamAccount",
              groupName: null,
              upstreamAccountId: 108,
              upstreamAccountName: "Codex Pro - Tokyo",
            },
          },
        ),
        createConversation(
          "pck-story-manual-binding-long-account",
          [
            createPreview({
              id: 8301,
              invokeId: "story-manual-binding-long-account",
              occurredAt: createRelativeStoryIso(-65_000),
              status: "completed",
              upstreamAccountId: 219,
            }),
          ],
          {
            manualBinding: {
              bindingKind: "upstreamAccount",
              groupName: null,
              upstreamAccountId: 219,
              upstreamAccountName:
                "paisleeeinar5710 Team sandbox workflow monitor with an intentionally long account label",
            },
          },
        ),
      ])}
      theme="vibe-dark"
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const badgeButtons = canvas.getAllByTestId(
      "dashboard-working-conversation-manual-binding-badge",
    );
    await expect(badgeButtons[0]).toHaveTextContent("CIII");
    await expect(badgeButtons[1]).toHaveTextContent("Codex Pro - Tokyo");
    await expect(badgeButtons[2]).toHaveAttribute(
      "title",
      expect.stringContaining(
        "paisleeeinar5710 Team sandbox workflow monitor with an intentionally long account label",
      ),
    );

    await userEvent.click(badgeButtons[0]);

    await expect(
      within(document.body).getByRole("tab", { name: /设置|settings/i }),
    ).toHaveAttribute("aria-selected", "true");
  },
};

export const RunningOnlyConversation: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: (args) => (
    <DashboardWorkingConversationsSection
      {...args}
      cards={buildCards(createRunningOnlyResponse())}
    />
  ),
  play: async ({ canvasElement }) => {
    const currentSlot = (
      await within(canvasElement).findAllByTestId("dashboard-working-conversation-slot")
    ).find((slot) => slot.getAttribute("data-slot-kind") === "current");
    if (!currentSlot) {
      throw new Error("missing current slot");
    }
    const currentSlotHeader = await within(currentSlot).findByTestId(
      "dashboard-working-conversation-slot-header",
    );
    expect(currentSlotHeader).toHaveClass("grid");
    expect(currentSlotHeader).toHaveClass("grid-cols-[minmax(0,1fr)_auto]");
    await within(currentSlotHeader).findByTestId("invocation-phase-badge");

    const phaseLabels = Array.from(
      currentSlot.querySelectorAll('[data-testid="invocation-phase-badge"]'),
    );
    expect(phaseLabels.length).toBeGreaterThanOrEqual(1);
    for (const phaseLabel of phaseLabels) {
      expect(phaseLabel.className).toContain("inline-flex");
      expect(phaseLabel.className).toMatch(/\brounded-full\b/);
      expect(phaseLabel.getAttribute("data-phase-label-visible")).toBe("false");
      expect(phaseLabel.getAttribute("data-phase-motion")).toBe("dynamic");
      expect(phaseLabel.className).not.toMatch(/\bborder/);
    }
    const respondingBadge = currentSlotHeader.querySelector(
      '[data-testid="invocation-phase-badge"][data-phase="responding"]',
    );
    expect(respondingBadge).not.toBeNull();
    const respondingIcon = respondingBadge?.querySelector('[data-testid="invocation-phase-icon"]');
    expect(respondingIcon).not.toBeNull();
    expect(respondingIcon?.className).toContain("animate-spin");
    const measuredTtft = await within(currentSlot).findByTestId("dashboard-compact-latency-ttft");
    const measuredResponse = await within(currentSlot).findByTestId(
      "dashboard-compact-latency-response",
    );
    expect(measuredTtft).toHaveTextContent("0.9s");
    expect(measuredResponse).not.toHaveTextContent("--");
  },
};

export const RequestingConversation: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: (args) => (
    <DashboardWorkingConversationsSection
      {...args}
      cards={buildCards(createRequestingOnlyResponse())}
    />
  ),
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing current slot");
    }
    const requestingBadge = currentSlot.querySelector(
      '[data-testid="invocation-phase-badge"][data-phase="requesting"]',
    );
    if (!(requestingBadge instanceof HTMLElement)) {
      throw new Error("missing requesting phase badge");
    }
    await expect(requestingBadge).toHaveAttribute("data-phase-label-visible", "false");
    await expect(requestingBadge).toHaveAttribute("data-phase-motion", "dynamic");
    const requestingIcon = requestingBadge.querySelector('[data-testid="invocation-phase-icon"]');
    if (!(requestingIcon instanceof HTMLElement)) {
      throw new Error("missing requesting phase icon");
    }
    await expect(requestingIcon.className).toContain("animate-invocation-phase-requesting");
    await expect(currentSlot).not.toHaveTextContent(/请求中|Requesting/);
    expect(
      await within(currentSlot).findByTestId("dashboard-compact-latency-request"),
    ).toBeInTheDocument();
    expect(currentSlot.querySelector('[data-testid="dashboard-compact-latency-ttft"]')).toBeNull();
    expect(
      currentSlot.querySelector('[data-testid="dashboard-compact-latency-response"]'),
    ).toBeNull();
  },
};

export const PoolRoutingAccountStates: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={createPoolRoutingAccountStatesResponse()} />,
  parameters: {
    docs: {
      description: {
        story:
          "Dashboard working-conversation state gallery for pool routing account attribution: the running concrete upstream account breathes in primary text, the pending no-account slot keeps the neutral pool-routing fallback, and the terminal account stays static.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountButtons = await canvas.findAllByRole("button", {
      name: "pool-alpha@example.com",
    });
    const runningAccount = requireFixture(accountButtons[0]);
    await expect(runningAccount.className).toContain("invocation-account-routing-in-progress");
    await expect(canvas.getByText(/号池路由中|pool routing/i)).toBeInTheDocument();

    const terminalAccount = accountButtons[accountButtons.length - 1];
    await expect(terminalAccount.className).not.toContain("invocation-account-routing-in-progress");

    await userEvent.click(runningAccount);
    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain(
        "Mock shared account detail drawer used to verify",
      );
    });
  },
};

export const FailedStatusIconDedup: Story = {
  tags: ["test"],
  args: {
    activeRange: "today",
    cards: buildCards(failedStatusDedupResponse),
    isLoading: false,
    error: null,
  },
  play: async ({ canvasElement }) => {
    const currentSlot = canvasElement.querySelector(
      '[data-testid="dashboard-working-conversation-slot"][data-slot-kind="current"]',
    );
    if (!(currentSlot instanceof HTMLElement)) {
      throw new Error("missing failed current slot");
    }
    const slotHeader = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-slot-header"]',
    );
    if (!(slotHeader instanceof HTMLElement)) {
      throw new Error("missing failed slot header");
    }
    const statusIcon = slotHeader.querySelector(
      '[data-testid="dashboard-inline-invocation-status"]',
    );
    if (!(statusIcon instanceof HTMLElement)) {
      throw new Error("missing compact failed status icon");
    }
    await expect(statusIcon).toHaveAttribute("aria-label", expect.stringContaining("失败"));
    await expect(statusIcon).toHaveAttribute(
      "aria-label",
      expect.stringContaining("upstream gateway closed before first byte"),
    );
    expect(
      slotHeader.querySelectorAll('[aria-label*="upstream gateway closed before first byte"]'),
    ).toHaveLength(1);
    await expect(currentSlot).not.toHaveTextContent(/^失败$/);
    await expect(
      currentSlot.querySelector('[data-testid="invocation-error-summary"]'),
    ).toBeInTheDocument();
    await expect(currentSlot).not.toHaveTextContent(/错误|Error/);
  },
  parameters: {
    docs: {
      description: {
        story:
          "Failure slot compact status case that keeps exactly one owner-facing failed icon in the header while moving the collapsed error summary onto that single status affordance.",
      },
    },
  },
};

export const AccountPlanBadges: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(accountPlanBadgeResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable account-row polish case with long account names and compact plan badges. Enterprise is abbreviated to `Ent` while the full plan remains available in the badge title.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = Array.from(
      canvasElement.querySelectorAll('[data-testid="dashboard-working-conversation-account-plan"]'),
    );
    expect(badges.map((badge) => badge.textContent)).toEqual(
      expect.arrayContaining(["Ent", "Team", "Plus", "Free"]),
    );
    expect(badges.find((badge) => badge.textContent === "Ent")?.getAttribute("title")).toBe(
      "enterprise",
    );
  },
};

export const TransportBadgeMixed: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(transportBadgeResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Mixed transport working-conversation cards. The current WebSocket invocation shows `WS` between the status badge and endpoint pill; the previous HTTP slot stays unbadged.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const badges = canvasElement.querySelectorAll('[data-testid="invocation-transport-badge"]');
    expect(badges.length).toBeGreaterThanOrEqual(1);
    expect(
      Array.from(badges).every(
        (badge) => badge.querySelector('[aria-hidden="true"]')?.textContent === "WS",
      ),
    ).toBe(true);
  },
};

export const ModelRoutingMismatch: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(
      createResponse([
        createConversation("pck-model-routing", [
          createPreview({
            id: 9701,
            invokeId: "inv_dashboard_model_routing",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "success",
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }),
        ]),
      ]),
    ),
    isLoading: false,
    error: null,
  },
};

export const InvocationDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={failedClickableResponse}
      initialSelection={{
        promptCacheKey: "pck-failed-clickable",
        slotKind: "current",
      }}
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Dashboard card section with the new invocation detail drawer opened by default, backed by stable request-id lookups and mock response-body detail data.",
      },
    },
  },
};

export const ModelRoutingDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-model-routing-drawer", [
          createPreview({
            id: 9801,
            invokeId: "inv_dashboard_model_routing_drawer",
            occurredAt: "2026-04-04T10:06:00Z",
            status: "success",
            model: "gpt-5.5",
            requestModel: "gpt-5.4",
            responseModel: "gpt-5.5",
          }),
        ]),
      ])}
      initialSelection={{
        promptCacheKey: "pck-model-routing-drawer",
        slotKind: "current",
      }}
    />
  ),
};

export const InterruptedRecoveryDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={interruptedRecoveryResponse}
      initialSelection={{
        promptCacheKey: "pck-interrupted-recovery",
        slotKind: "current",
      }}
    />
  ),
  parameters: {
    docs: {
      description: {
        story:
          "Recovered interrupted invocation that is immediately queryable from the dashboard drawer and keeps the dedicated interrupted status badge.",
      },
    },
  },
};

export const AssignedAccountFailureSemantics: Story = {
  args: {
    activeRange: "today",
    cards: buildCards(assignedAccountFailureSemanticsResponse),
    isLoading: false,
    error: null,
  },
  parameters: {
    docs: {
      description: {
        story:
          "Current dashboard working-conversation cards proving that assigned-account failures keep the concrete upstream account label, while true no-account failures alone fall back to the unassigned-account label.",
      },
    },
  },
};

export const FailedWithClickableAccount: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => <DrawerPreviewStory response={failedClickableResponse} />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountButtons = await canvas.findAllByRole("button", {
      name: /pool-account-77@example.com/i,
    });
    const accountButton = accountButtons[0];

    await userEvent.click(accountButton);

    await waitFor(() => {
      expect(document.body.textContent ?? "").toContain(
        "Mock shared account detail drawer used to verify",
      );
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent("account:77:overview");
  },
};

export const SequenceButtonOpensConversationHistory: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={fixtures.historyInvocationsByPromptCacheKey}
        theme="vibe-dark"
      />
    );
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const sequenceButton = await canvas.findByTestId(
      "dashboard-working-conversation-sequence-button",
    );

    sequenceButton.focus();
    await userEvent.keyboard("{Enter}");

    await waitFor(() => {
      expect(
        document.body.querySelector('[data-testid="story-drawer-state"]')?.textContent,
      ).toContain("conversation:pck-dashboard-history-realistic");
    });
    await expect(canvas.getByTestId("story-drawer-state")).toHaveTextContent(
      "conversation:pck-dashboard-history-realistic",
    );
    expect(document.body.textContent ?? "").toContain(sequenceButton.textContent ?? "");
    expect(document.body.textContent ?? "").toContain("pck-dashboard-history-realistic");
    await expect(
      within(document.body).getByText(/对话详情|Conversation details/i),
    ).toBeInTheDocument();
    await waitFor(() => {
      expect(document.body.textContent ?? "").toMatch(/共 316 条保留调用记录|316 retained calls/i);
    });
    const dialog = within(document.body).getByRole("dialog");
    expect(within(dialog).queryByRole("button", { name: "今日" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "昨日" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "24 小时" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "7 日" })).toBeNull();
    expect(within(dialog).queryByRole("button", { name: "历史" })).toBeNull();
    await waitFor(() => {
      const fetchLog =
        (window as typeof window & { __dashboardStoryFetchLog?: string[] })
          .__dashboardStoryFetchLog ?? [];
      expect(
        fetchLog.some(
          (entry) =>
            entry.startsWith("/api/invocations?") &&
            entry.includes("promptCacheKey=pck-dashboard-history-realistic") &&
            entry.includes("page=2") &&
            entry.includes("snapshotId=1"),
        ),
      ).toBe(true);
    });
  },
  parameters: {
    docs: {
      description: {
        story:
          "Only the compact conversation sequence id is a hot zone for opening the full retained conversation history drawer; invocation slots still open single-call diagnostics.",
      },
    },
  },
};

export const ConversationHistoryDrawerOpen: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={fixtures.historyInvocationsByPromptCacheKey}
        initialConversationKey="pck-dashboard-history-realistic"
        theme="vibe-dark"
      />
    );
  },
  parameters: {
    docs: {
      description: {
        story:
          "Stable opened state for the full retained conversation history drawer, including the production-style activity chart and dark floating tooltip surface.",
      },
    },
  },
};

export const ConversationHistoryPageMobile: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => {
    const fixtures = buildDashboardHistoryEvidenceFixtures();
    return (
      <DrawerPreviewStory
        response={fixtures.dashboardResponse}
        historyInvocationsByPromptCacheKey={fixtures.historyInvocationsByPromptCacheKey}
        initialConversationKey="pck-dashboard-history-realistic"
        initialConversationTab="settings"
        conversationPresentation="page"
        theme="vibe-dark"
      />
    );
  },
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "Compact page presentation for the retained conversation history workspace, using the same URL-backed content hierarchy as the desktop drawer.",
      },
    },
  },
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText(/对话详情|Conversation details/i)).toBeInTheDocument();
    await expect(canvas.getByRole("tab", { name: /设置|settings/i })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await expect(within(document.body).queryByRole("dialog")).toBeNull();
  },
};

export const UpstreamAccountTab: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={createResponse([
        createConversation("pck-story-upstream-account", [
          createPreview({
            id: 9801,
            invokeId: "story-working-invoke",
            occurredAt: "2026-04-04T10:05:00Z",
            status: "running",
            upstreamAccountId: 42,
            upstreamAccountName: "Pool Alpha",
          }),
        ]),
      ])}
      upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
    />
  ),
  play: ({ canvasElement }) => assertUpstreamAccountTabStory(canvasElement),
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Dashboard workspace section switched to the upstream-account tab, showing one enlarged active-account card with account-level KPIs and the dynamic recent invocation window in the selected range, including lightweight short conversation identity chips and request/response model mismatch rows.",
      },
    },
  },
};

export const SummaryThresholdTones: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <DrawerPreviewStory
      response={summaryThresholdResponse}
      upstreamAccountActivity={summaryThresholdUpstreamActivity}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const currentSlot = await canvas.findByTestId("dashboard-working-conversation-slot");
    const currentHit = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-hit"]',
    );
    const currentCost = currentSlot.querySelector(
      '[data-testid="dashboard-working-conversation-usage-cost"]',
    );
    if (!(currentHit instanceof HTMLElement) || !(currentCost instanceof HTMLElement)) {
      throw new Error("missing threshold conversation summary fields");
    }
    await expect(currentHit).toHaveTextContent("Hit 95.5%");
    await expect(currentHit).toHaveAttribute("data-summary-tone", "default");
    await expect(currentCost).toHaveTextContent("$0.0586");
    await expect(currentCost).toHaveAttribute("data-summary-tone", "default");

    const slots = canvasElement.querySelectorAll(
      '[data-testid="dashboard-working-conversation-slot"]',
    );
    const warningSlot = slots[1];
    const errorSlot = slots[2];
    if (!(warningSlot instanceof HTMLElement) || !(errorSlot instanceof HTMLElement)) {
      throw new Error("missing threshold comparison slots");
    }
    await expect(
      warningSlot.querySelector('[data-testid="dashboard-working-conversation-usage-hit"]'),
    ).toHaveAttribute("data-summary-tone", "warning");
    await expect(
      warningSlot.querySelector('[data-testid="dashboard-working-conversation-usage-cost"]'),
    ).toHaveAttribute("data-summary-tone", "warning");
    await expect(
      errorSlot.querySelector('[data-testid="dashboard-working-conversation-usage-hit"]'),
    ).toHaveAttribute("data-summary-tone", "error");
    await expect(
      errorSlot.querySelector('[data-testid="dashboard-working-conversation-usage-cost"]'),
    ).toHaveAttribute("data-summary-tone", "error");

    const accountTab = await canvas.findByRole("tab", { name: "上游账号" });
    await userEvent.click(accountTab);
    const recentRows = await canvas.findAllByTestId("dashboard-upstream-account-recent-row");
    await expect(
      recentRows[0]?.querySelector('[data-testid="dashboard-upstream-account-recent-summary-hit"]'),
    ).toHaveAttribute("data-summary-tone", "warning");
    await expect(
      recentRows[0]?.querySelector(
        '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
      ),
    ).toHaveAttribute("data-summary-tone", "warning");
    await expect(
      recentRows[1]?.querySelector('[data-testid="dashboard-upstream-account-recent-summary-hit"]'),
    ).toHaveAttribute("data-summary-tone", "error");
    await expect(
      recentRows[1]?.querySelector(
        '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
      ),
    ).toHaveAttribute("data-summary-tone", "error");
    await expect(
      recentRows[2]?.querySelector('[data-testid="dashboard-upstream-account-recent-summary-hit"]'),
    ).toHaveTextContent("Hit 90%");
    await expect(
      recentRows[2]?.querySelector(
        '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
      ),
    ).toHaveTextContent("$0.1000");
    await expect(
      recentRows[3]?.querySelector('[data-testid="dashboard-upstream-account-recent-summary-hit"]'),
    ).toHaveTextContent("Hit 50%");
    await expect(
      recentRows[3]?.querySelector(
        '[data-testid="dashboard-upstream-account-recent-summary-cost"]',
      ),
    ).toHaveTextContent("$0.5000");
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Threshold gallery for conversation slots and upstream-account recent rows, showing default, warning, error, and strict boundary states with the unified Hit / Token / cost summary contract.",
      },
    },
  },
};

export const ConversationTabWithUpstreamNetworkSpeed: Story = {
  args: {
    activeRange: "today",
    cards: [],
    isLoading: false,
    error: null,
  },
  render: () => (
    <ForcedWorkspaceViewStory view="conversations">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-conversation-network-speed", [
            createPreview({
              id: 9802,
              invokeId: "story-conversation-invoke",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("当前对话 1 条")).toBeInTheDocument();
    const totalNetworkSpeed = await canvas.findByTestId(
      "dashboard-upstream-account-total-network-speed",
    );
    await expect(totalNetworkSpeed).toHaveTextContent("46");
    await expect(totalNetworkSpeed).toHaveTextContent("KiB/s");
    await expect(totalNetworkSpeed).toHaveTextContent("214");
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Conversation workspace header with aggregate upstream upload and download throughput rendered beside the live-count badge.",
      },
    },
  },
};

export const UpstreamAccountWarningSuccess: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={warningSuccessConversationResponse}
        upstreamAccountActivity={createWarningSuccessUpstreamAccountActivityResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();
    const statusNode = await canvas.findByTestId("dashboard-inline-invocation-status");
    await expect(statusNode.getAttribute("title")).toBeNull();
    await expect(statusNode.getAttribute("aria-label") ?? "").toContain("警告成功");
    await userEvent.hover(statusNode);
    await expect(within(document.body).getByRole("tooltip")).toHaveTextContent("警告成功");
    await expect(canvas.getByText("story-account-1")).toBeInTheDocument();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Upstream-account workspace card with a recent invocation rendered as warning-success, preserving success-like placement while exposing the dedicated owner-facing label.",
      },
    },
  },
};

export const UpstreamAccountRecentLayout: Story = {
  tags: ["test"],
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createUpstreamAccountRecentLayoutStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const accountGrid = await canvas.findByTestId("dashboard-upstream-account-grid");
    const accountCards = canvas.getAllByTestId("dashboard-upstream-account-card");
    const errorCard = accountCards.find((card) => card.dataset.accountKey === "42");
    const normalCard = accountCards.find((card) => card.dataset.accountKey === "87");
    if (!(errorCard instanceof HTMLElement) || !(normalCard instanceof HTMLElement)) {
      throw new Error("missing paired account cards");
    }

    const recentRow = errorCard.querySelector(
      '[data-testid="dashboard-upstream-account-recent-row"]',
    );
    const recentList = errorCard.querySelector(
      '[data-testid="dashboard-upstream-account-recent-list"]',
    );
    const detailsRow = recentRow?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-details-row"]',
    );
    const metaLine = recentRow?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-meta-line"]',
    );
    const summaryLine = recentRow?.querySelector(
      '[data-testid="dashboard-upstream-account-recent-summary-line"]',
    );
    if (
      !(recentRow instanceof HTMLElement) ||
      !(recentList instanceof HTMLElement) ||
      !(detailsRow instanceof HTMLElement) ||
      !(metaLine instanceof HTMLElement) ||
      !(summaryLine instanceof HTMLElement)
    ) {
      throw new Error("missing upstream account recent layout row");
    }
    await expect(
      recentRow.querySelector('[data-testid="dashboard-compact-latency-response"]'),
    ).toHaveTextContent("--");
    const recentLatencyPills = recentRow.querySelector(
      '[data-testid="dashboard-compact-latency-pills"]',
    );
    await expect(recentLatencyPills).toHaveClass("gap-1");
    await expect(recentLatencyPills).not.toHaveTextContent(/\d\s+s/);

    expect(accountGrid.className).toContain("items-start");
    expect(errorCard.className).not.toContain("h-full");
    expect(errorCard.className).not.toContain("desktop1660:min-h-[31.5rem]");
    expect(recentList.className).toContain("content-start");
    expect(recentList.className).not.toContain("flex-1");
    expect(recentList.className).not.toContain("auto-rows-fr");
    expect(detailsRow.className).toContain("grid-cols-1");
    const expectedDetailsLayout = window.innerWidth >= 640 ? "split" : "stacked";
    expect(detailsRow.dataset.layout).toBe(expectedDetailsLayout);
    if (expectedDetailsLayout === "split") {
      expect(detailsRow.className).toContain("sm:grid-cols-2");
    } else {
      expect(detailsRow.className).not.toContain("sm:grid-cols-2");
    }
    expect(
      metaLine.compareDocumentPosition(summaryLine) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);
    if (expectedDetailsLayout === "split") {
      expect(summaryLine.className).toContain("sm:justify-end");
    } else {
      expect(summaryLine.className).not.toContain("sm:justify-end");
    }
    expect(errorCard.querySelector('[data-testid="invocation-error-summary"]')).toBeTruthy();
    expect(normalCard.querySelector('[data-testid="invocation-error-summary"]')).toBeNull();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Desktop account-pair layout: the timestamp/model metadata stays left of the Hit, Token, and Cost summary, while only the failed account grows for its truncated error summary.",
      },
    },
  },
};

export const UpstreamAccountRecentLayoutMobile: Story = {
  ...UpstreamAccountRecentLayout,
  parameters: {
    viewport: { defaultViewport: "mobile390" },
    docs: {
      description: {
        story:
          "Narrow account layout falls back to metadata followed by the compact summary, without horizontal overflow or hiding the error summary.",
      },
    },
  },
};

export const UpstreamAccountImageEditEndpoint: Story = {
  tags: ["test"],
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createImageEditEndpointUpstreamAccountActivityStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  globals: {
    themeMode: "light",
  },
  parameters: {
    a11y: {
      options: {
        rules: {
          "color-contrast": { enabled: false },
          "nested-interactive": { enabled: false },
        },
      },
      config: {
        rules: [
          { id: "color-contrast", enabled: false },
          { id: "nested-interactive", enabled: false },
        ],
      },
    },
  },
  play: async ({ canvasElement }) => {
    await assertImageEditEndpointRecentRow(canvasElement);
  },
};

export const UpstreamAccountImageEditEndpointDark: Story = {
  ...UpstreamAccountImageEditEndpoint,
  tags: ["test"],
  globals: {
    themeMode: "dark",
  },
};

export const UpstreamAccountInitialSkeleton: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={null}
        upstreamAccountActivityLoading
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-upstream-account-grid-skeleton")).toBeVisible();
    await expect(canvas.getByText("账号加载中")).toBeInTheDocument();
    await expect(canvas.queryByText("当前范围内暂无活动上游账号。")).toBeNull();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story: "Initial account-view frame with a layout-stable card skeleton.",
      },
    },
  },
};

export const UpstreamAccountSummaryWithRecentLoading: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
        upstreamAccountRecentLoading
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByTestId("dashboard-upstream-account-card")).toBeVisible();
    await expect(canvas.getAllByTestId("dashboard-upstream-account-recent-skeleton")).toHaveLength(
      4,
    );
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story: "Account summary cards remain usable while recent rows load.",
      },
    },
  },
};

export const UpstreamAccountRecentFailure: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
        upstreamAccountRecentError="recent request failed"
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("最近调用加载失败。")).toBeVisible();
    await expect(canvas.getByRole("button", { name: "重试最近调用" })).toBeVisible();
    await expect(canvas.getByTestId("dashboard-upstream-account-card")).toBeVisible();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story: "Recent-row failure is local to each retained summary card.",
      },
    },
  },
};

export const UpstreamAccountRefreshing: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
        upstreamAccountActivityRefreshing
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(async () => {
      await expect(canvas.getByRole("status", { name: "正在更新账号汇总" })).toBeVisible();
    });
    await expect(canvas.getByTestId("dashboard-upstream-account-card")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-upstream-account-refresh-text")).toBeVisible();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Range refresh keeps the previous cards visible until replacement while the header status collapses to a lightweight spinner + label ahead of the count badge, without reserving idle whitespace or inserting a new row above the account grid.",
      },
    },
  },
};

export const UpstreamAccountRefreshingMobile: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
        upstreamAccountActivityRefreshing
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await waitFor(async () => {
      await expect(canvas.getByRole("status", { name: "正在更新账号汇总" })).toBeVisible();
    });
    await expect(canvas.getByTestId("dashboard-upstream-account-card")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-working-conversations-badges")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-upstream-account-refresh-spinner")).toBeVisible();
    await expect(canvas.getByTestId("dashboard-upstream-account-refresh-text")).not.toBeVisible();
  },
  parameters: {
    viewport: { defaultViewport: "mobile430" },
    docs: {
      description: {
        story:
          "Mobile refresh keeps the account card visible while the header collapses the visual treatment to spinner-only; the accessible status remains intact, but no text badge or idle placeholder consumes an extra slot.",
      },
    },
  },
};

export const UpstreamAccountEmpty: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([])}
        upstreamAccountActivity={{
          ...createUpstreamAccountActivityStoryResponse(),
          accounts: [],
        }}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("当前范围内暂无活动上游账号。")).toBeVisible();
    await expect(canvas.queryByTestId("dashboard-upstream-account-grid-skeleton")).toBeNull();
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story: "True empty state after a successful zero-account summary response.",
      },
    },
  },
};

export const UpstreamAccountPhaseBreakdownStatic: Story = {
  args: UpstreamAccountTab.args,
  render: () => (
    <ForcedWorkspaceViewStory view="upstreamAccounts">
      <DrawerPreviewStory
        response={createResponse([
          createConversation("pck-story-upstream-account-static", [
            createPreview({
              id: 9841,
              invokeId: "story-working-static",
              occurredAt: "2026-04-04T10:05:00Z",
              status: "running",
              upstreamAccountId: 42,
              upstreamAccountName: "Pool Alpha",
            }),
          ]),
        ])}
        upstreamAccountActivity={createUpstreamAccountActivityStoryResponse()}
      />
    </ForcedWorkspaceViewStory>
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(canvas.getByText("当前活动账号 1 个")).toBeInTheDocument();
    const recentBreakdown = await canvas.findByTestId(
      "dashboard-upstream-account-recent-breakdown",
    );
    const phaseSegments = Array.from(
      recentBreakdown.querySelectorAll('[data-testid="invocation-phase-segment"]'),
    );
    expect(phaseSegments).toHaveLength(3);
    for (const phaseSegment of phaseSegments) {
      expect(phaseSegment.getAttribute("data-phase-motion")).toBe("static");
      const icon = phaseSegment.querySelector('[data-testid="invocation-phase-icon"]');
      expect(icon).toBeInstanceOf(HTMLElement);
      expect(icon?.className).not.toContain("animate-invocation-phase-requesting");
      expect(icon?.className).not.toContain("animate-pulse");
      expect(icon?.className).not.toContain("animate-spin");
    }
  },
  parameters: {
    viewport: { defaultViewport: "desktop1660" },
    docs: {
      description: {
        story:
          "Owner-facing static phase breakdown entry that opens directly on the upstream-account workspace view, so the queued/requesting/responding summary can be reviewed without relying on an interaction step first.",
      },
    },
  },
};
