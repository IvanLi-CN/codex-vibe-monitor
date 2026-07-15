import type { Meta, StoryObj } from "@storybook/react-vite";
import { expect, userEvent, within } from "storybook/test";
import {
  AccountPoolStoryRouter,
  type UpstreamAccountCreatePage,
  upstreamAccountCreateMetaBase,
} from "./UpstreamAccountCreatePage.story-common";

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: "Account Pool/Pages/Upstream Account Create/Imported OAuth",
} satisfies Meta<typeof UpstreamAccountCreatePage>;

export default meta;

type Story = StoryObj<typeof meta>;

function buildJwt(payload: Record<string, unknown>) {
  const encode = (value: Record<string, unknown>) =>
    btoa(JSON.stringify(value)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
  return `${encode({ alg: "none", typ: "JWT" })}.${encode(payload)}.signature`;
}

function renderImportedOauthStory(defaultGroupName = "production") {
  return (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts/new",
        search: "?mode=import",
        state: {
          draft: {
            import: {
              defaultGroupName,
            },
          },
        },
      }}
    />
  );
}

function renderImportedSessionStory(defaultGroupName = "production") {
  return (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts/new",
        search: "?mode=importSession",
        state: {
          draft: {
            import: {
              defaultGroupName,
            },
          },
        },
      }}
    />
  );
}

function buildPastedCredential(
  overrides?: Partial<{
    type: string;
    email: string;
    account_id: string;
    expired: string;
    _storybookStatus: string;
    _storybookDetail: string;
  }>,
) {
  return JSON.stringify({
    type: overrides?.type ?? "auth0",
    email: overrides?.email ?? "paste-story@duckmail.sbs",
    account_id: overrides?.account_id ?? "acct_paste_story",
    expired: overrides?.expired ?? "2026-03-20T00:00:00.000Z",
    access_token: "access-token",
    refresh_token: "refresh-token",
    id_token: buildJwt({
      email: overrides?.email ?? "paste-story@duckmail.sbs",
      auth: {
        chatgpt_account_id: overrides?.account_id ?? "acct_paste_story",
      },
    }),
    ...(overrides?._storybookStatus ? { _storybookStatus: overrides._storybookStatus } : {}),
    ...(overrides?._storybookDetail ? { _storybookDetail: overrides._storybookDetail } : {}),
  });
}

function buildPastedSession() {
  return JSON.stringify({
    user: {
      id: "user_session_story",
      email: "session-story@duckmail.sbs",
    },
    account: {
      id: "acct_session_story",
      planType: "plus",
    },
    accessToken: "access-token",
    sessionToken: "session-token",
    expires: "2026-08-06T14:29:36.155Z",
  });
}

function buildPastedSub2apiPackage() {
  return JSON.stringify({
    type: "sub2api-data",
    accounts: [
      {
        platform: "openai",
        type: "oauth",
        credentials: {
          email: "sub2api-story-one@duckmail.sbs",
          chatgpt_account_id: "acct_story_shared_k12",
          chatgpt_user_id: "user_story_one",
          plan_type: "k12",
          access_token: "access-token-one",
          refresh_token: "refresh-token-one",
          id_token: buildJwt({
            email: "sub2api-story-one@duckmail.sbs",
            auth: {
              chatgpt_account_id: "acct_story_shared_k12",
              chatgpt_user_id: "user_story_one",
              chatgpt_plan_type: "k12",
            },
          }),
          expires_at: "2026-03-20T00:00:00.000Z",
        },
      },
      {
        platform: "openai",
        type: "oauth",
        credentials: {
          email: "sub2api-story-two@duckmail.sbs",
          chatgpt_account_id: "acct_story_shared_k12",
          chatgpt_user_id: "user_story_two",
          plan_type: "k12",
          access_token: "access-token-two",
          refresh_token: "refresh-token-two",
          id_token: buildJwt({
            email: "sub2api-story-two@duckmail.sbs",
            auth: {
              chatgpt_account_id: "acct_story_shared_k12",
              chatgpt_user_id: "user_story_two",
              chatgpt_plan_type: "k12",
            },
          }),
          expires_at: "2026-03-20T00:00:00.000Z",
        },
      },
    ],
  });
}

async function uploadImportFixture(canvasElement: HTMLElement) {
  const canvas = within(canvasElement);
  const fileInput = canvasElement.querySelector('input[type="file"]');
  if (!(fileInput instanceof HTMLInputElement)) {
    throw new Error("missing imported oauth file input");
  }
  const file = new File(
    [
      JSON.stringify({
        type: "codex",
        email: "story-import@duckmail.sbs",
        account_id: "acct_story_import",
        expired: "2026-03-20T00:00:00.000Z",
        access_token: "access-token",
        refresh_token: "refresh-token",
        id_token: buildJwt({
          email: "story-import@duckmail.sbs",
          auth: {
            chatgpt_account_id: "acct_story_import",
          },
        }),
      }),
    ],
    "story-import@duckmail.sbs.json",
    { type: "application/json" },
  );
  await userEvent.upload(fileInput, file);
  await expect(canvas.getByText(/story-import@duckmail\.sbs\.json/i)).toBeInTheDocument();
}

export const ReadyToValidate: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await uploadImportFixture(canvasElement);
    await expect(canvas.getByRole("button", { name: /validate/i })).toBeEnabled();
  },
};

export const IdlePasteEditor: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    await expect(editor).toHaveValue("");
    await expect(canvas.getByText(/paste exactly one credential json object/i)).toBeInTheDocument();
  },
};

export const PasteInvalidEditable: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    await userEvent.click(editor);
    await userEvent.paste('[{"type":"codex"}]');
    await expect(
      canvas.getByText(
        /paste exactly one credential json object or one sub2api-data export object/i,
      ),
    ).toBeInTheDocument();
    await expect(editor).toHaveValue('[{"type":"codex"}]');
  },
};

export const PasteMultipleErrors: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    await userEvent.click(editor);
    await userEvent.paste(
      JSON.stringify({
        type: "auth0",
        access_token: "",
        id_token: "not-a-jwt",
        expired: 123,
      }),
    );
    await expect(canvas.getByText(/email is required/i)).toBeInTheDocument();
    await expect(canvas.getByText(/account_id is required/i)).toBeInTheDocument();
    await expect(canvas.getByText(/access_token is required/i)).toBeInTheDocument();
    await expect(canvas.getByText(/id_token must be a valid jwt/i)).toBeInTheDocument();
  },
};

export const PasteAddedToQueue: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    await userEvent.click(editor);
    await userEvent.paste(buildPastedCredential());
    await expect(canvas.getByText(/pasted credential #1\.json/i)).toBeInTheDocument();
    await expect(editor).toHaveValue("");
  },
};

export const PasteSub2apiPackageAddedToQueue: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    await userEvent.click(editor);
    await userEvent.paste(buildPastedSub2apiPackage());
    await expect(canvas.getByText(/sub2api-story-one@duckmail\.sbs/i)).toBeInTheDocument();
    await expect(canvas.getByText(/sub2api-story-two@duckmail\.sbs/i)).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: /validate/i })).toBeEnabled();
  },
};

export const PasteDuplicateBlocked: Story = {
  render: () => renderImportedOauthStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste one credential json/i);
    const credential = buildPastedCredential();
    await userEvent.click(editor);
    await userEvent.paste(credential);
    await expect(canvas.getByText(/pasted credential #1\.json/i)).toBeInTheDocument();
    await userEvent.click(editor);
    await userEvent.paste(credential);
    await expect(canvas.getByText(/already queued/i)).toBeInTheDocument();
    await expect(canvas.getAllByText(/pasted credential #1\.json/i)).toHaveLength(1);
  },
};

export const WebSessionPasteAddedToQueue: Story = {
  render: () => renderImportedSessionStory(),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const editor = canvas.getByLabelText(/paste web session json/i);
    await expect(canvas.getByText(/chatgpt web session/i)).toBeInTheDocument();
    await userEvent.click(editor);
    await userEvent.paste(buildPastedSession());
    await expect(canvas.getByText(/pasted session #1/i)).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: /validate/i })).toBeEnabled();
  },
};

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => renderImportedOauthStory("staging"),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await uploadImportFixture(canvasElement);
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: /validate/i })).toBeDisabled();
  },
};
