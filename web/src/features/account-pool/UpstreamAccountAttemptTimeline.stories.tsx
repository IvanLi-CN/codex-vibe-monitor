import type { Meta, StoryObj } from "@storybook/react-vite";
import { type ReactNode, useEffect } from "react";
import { MemoryRouter } from "react-router-dom";
import { I18nProvider } from "../../i18n";
import { FullPageStorySurface } from "../../storybook/storybookPageHelpers";
import { UpstreamAccountAttemptTimeline } from "./UpstreamAccountAttemptTimeline";

function StorySurface({ children }: { children: ReactNode }) {
  return (
    <div className="bg-[#f6f1e7] px-6 py-6 text-base-content sm:px-8">
      <div className="mx-auto max-w-6xl rounded-[28px] border border-base-300/70 bg-base-200 px-6 py-6 shadow-sm">
        {children}
      </div>
    </div>
  );
}

function AttemptTimelinePageSurface({ children }: { children: ReactNode }) {
  return (
    <FullPageStorySurface>
      <div className="mx-auto max-w-7xl space-y-5">
        <header className="rounded-[28px] border border-base-300/70 bg-base-100/85 px-6 py-6 shadow-sm">
          <p className="text-xs font-semibold uppercase tracking-[0.22em] text-base-content/55">
            Account Pool Review
          </p>
          <h1 className="mt-3 text-3xl font-semibold text-base-content">
            Upstream attempt compression evidence
          </h1>
          <p className="mt-2 max-w-3xl text-sm leading-6 text-base-content/70">
            Page-level fallback surface for reviewing retry attempt compression ratio and
            approximate upstream transfer bytes.
          </p>
        </header>

        <section className="rounded-[32px] border border-base-300/70 bg-base-100/82 px-6 py-6 shadow-sm">
          {children}
        </section>
      </div>
    </FullPageStorySurface>
  );
}

function AttemptTimelineFetchMock({ accountId }: { accountId: number }) {
  useEffect(() => {
    const originalFetch = globalThis.fetch;
    globalThis.fetch = async (input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      if (url.includes("/api/pool/forward-proxy-binding-nodes")) {
        return new Response(
          JSON.stringify([
            {
              key: "jp-edge-01",
              source: "manual",
              displayName: "JP Edge 01",
              protocolLabel: "HTTP",
              egressIp: null,
              egressIpCheckedAt: null,
              egressIpProvider: null,
              egressIpError: null,
              egressIpErrorAt: null,
              penalized: false,
              selectable: true,
              last24h: [],
            },
          ]),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      if (
        url.includes(`/api/pool/upstream-accounts/${accountId}/call-attempts/locate`) ||
        url.includes(`/api/pool/upstream-accounts/${accountId}/call-attempts?`)
      ) {
        return new Response(
          JSON.stringify({
            items: [
              {
                attemptId: "4V7MYPJG",
                invokeId: "POOLCALL001",
                occurredAt: "2026-07-11T12:00:00.000Z",
                endpoint: "/v1/responses",
                upstreamAccountId: accountId,
                requestModel: "gpt-5.4",
                responseModel: "gpt-5.4-2026-07-01",
                proxyBindingKeySnapshot: "jp-edge-01",
                attemptIndex: 1,
                distinctAccountIndex: 0,
                sameAccountRetryIndex: 0,
                status: "http_failure",
                phase: "failed",
                httpStatus: 500,
                downstreamHttpStatus: 502,
                failureKind: "upstream_response_failed",
                errorMessage: "upstream returned an oversized diagnostic payload",
                connectLatencyMs: 120,
                firstByteLatencyMs: 480,
                streamLatencyMs: 810,
                downstreamRequestContentEncoding: "gzip",
                upstreamRequestCompressionAlgorithm: "zstd",
                upstreamRequestCompressionMode: "recompressed",
                logicalBodyBytes: 1000,
                transmittedBodyBytes: 580,
                savedBytes: 420,
                ratioPct: -42,
                approxUploadBytes: 644,
                approxDownloadBytes: 812,
                upstreamRequestId: "req_upstream_123",
                upstreamRouteKey: "route-tokyo-primary",
                createdAt: "2026-07-11T12:00:00.000Z",
              },
            ],
            total: 1,
            page: 1,
            pageSize: 50,
          }),
          {
            status: 200,
            headers: {
              "Content-Type": "application/json",
            },
          },
        );
      }
      return originalFetch(input, init);
    };
    return () => {
      globalThis.fetch = originalFetch;
    };
  }, [accountId]);

  return null;
}

const meta = {
  title: "Account Pool/Components/Upstream Account Attempt Timeline",
  component: UpstreamAccountAttemptTimeline,
  decorators: [
    (Story, context) => (
      <I18nProvider>
        <MemoryRouter>
          {context.parameters.pageSurface ? (
            <Story />
          ) : (
            <StorySurface>
              <Story />
            </StorySurface>
          )}
        </MemoryRouter>
      </I18nProvider>
    ),
  ],
  parameters: {
    viewport: { defaultViewport: "desktop1280" },
  },
} satisfies Meta<typeof UpstreamAccountAttemptTimeline>;

export default meta;

type Story = StoryObj<typeof meta>;

export const FocusedFailureAttempt: Story = {
  args: {
    accountId: 101,
    focusedAttemptId: "4V7MYPJG",
  },
  decorators: [
    (Story) => (
      <>
        <AttemptTimelineFetchMock accountId={101} />
        <Story />
      </>
    ),
  ],
};

export const FocusedFailureAttemptPage: Story = {
  ...FocusedFailureAttempt,
  parameters: {
    layout: "fullscreen",
    viewport: { defaultViewport: "desktop1660" },
    pageSurface: true,
  },
  render: (args) => (
    <AttemptTimelinePageSurface>
      <UpstreamAccountAttemptTimeline
        accountId={args.accountId ?? 101}
        focusedAttemptId={args.focusedAttemptId ?? "4V7MYPJG"}
      />
    </AttemptTimelinePageSurface>
  ),
};
