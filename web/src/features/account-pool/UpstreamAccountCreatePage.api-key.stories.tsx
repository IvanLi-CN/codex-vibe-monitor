import type { Meta, StoryObj } from "@storybook/react-vite";
import { useEffect, useRef } from "react";
import { expect, userEvent, within } from "storybook/test";
import { UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY } from "../../lib/upstreamAccountGroups";
import {
  AccountPoolStoryRouter,
  type UpstreamAccountCreatePage,
  upstreamAccountCreateMetaBase,
} from "./UpstreamAccountCreatePage.story-common";

const meta = {
  ...upstreamAccountCreateMetaBase,
  title: "Account Pool/Pages/Upstream Account Create/API Key",
} satisfies Meta<typeof UpstreamAccountCreatePage>;

export default meta;

type Story = StoryObj<typeof meta>;

function RememberedApiKeyGroupStoryRouter() {
  const restoreRef = useRef<null | (() => void)>(null);

  if (restoreRef.current == null && typeof window !== "undefined") {
    const previousValue = window.localStorage.getItem(
      UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY,
    );
    window.localStorage.setItem(
      UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY,
      JSON.stringify({ groupName: "production" }),
    );
    restoreRef.current = () => {
      if (previousValue == null) {
        window.localStorage.removeItem(UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY);
        return;
      }
      window.localStorage.setItem(
        UPSTREAM_ACCOUNT_CREATE_API_KEY_LAST_GROUP_STORAGE_KEY,
        previousValue,
      );
    };
  }

  useEffect(() => {
    return () => {
      restoreRef.current?.();
      restoreRef.current = null;
    };
  }, []);

  return <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=apiKey" />;
}

export const EmailDerivedName: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=apiKey" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const displayName = canvas.getByLabelText(/display name/i) as HTMLInputElement;
    const email = canvas.getByLabelText(/^email$/i);

    await userEvent.type(email, "first@storybook.example.com");
    await expect(displayName.value).toBe("first@storybook.example.com");
    await userEvent.clear(email);
    await userEvent.type(email, "second@storybook.example.com");
    await expect(displayName.value).toBe("second@storybook.example.com");

    await userEvent.clear(displayName);
    await userEvent.type(displayName, "Manual Alias");
    await userEvent.clear(email);
    await userEvent.type(email, "manual@storybook.example.com");
    await expect(displayName.value).toBe("Manual Alias");
  },
};

export const Default: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=apiKey" />
  ),
};

export const RememberedSuccessfulGroup: Story = {
  render: () => <RememberedApiKeyGroupStoryRouter />,
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    const groupInput = canvasElement.querySelector('input[name="apiKeyGroupName"]');
    if (!(groupInput instanceof HTMLInputElement)) {
      throw new Error("missing API Key group hidden input");
    }

    await expect(groupInput.value).toBe("production");
    await expect(canvas.getByRole("combobox")).toHaveTextContent(/production/i);
  },
};

export const NameConflict: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts/new",
        search: "?mode=apiKey",
        state: {
          draft: {
            apiKey: {
              displayName: " team key - staging ",
              groupName: "staging",
              note: "Conflicts with an existing API Key account name.",
              apiKeyValue: "sk-storybookduplicate1234",
              primaryLimit: "120",
              secondaryLimit: "500",
              limitUnit: "requests",
            },
          },
        },
      }}
    />
  ),
};

export const BlockedByUnselectableGroupProxy: Story = {
  render: () => (
    <AccountPoolStoryRouter
      initialEntry={{
        pathname: "/account-pool/upstream-accounts/new",
        search: "?mode=apiKey",
        state: {
          draft: {
            apiKey: {
              displayName: "Staging Key",
              groupName: "staging",
              apiKeyValue: "sk-storybookstaging1234",
            },
          },
        },
      }}
    />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await expect(
      canvas.getByText(/group "staging" does not have any selectable bound proxy nodes\./i),
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: /create api key account/i })).toBeDisabled();
  },
};

export const InvalidUpstreamUrl: Story = {
  render: () => (
    <AccountPoolStoryRouter initialEntry="/account-pool/upstream-accounts/new?mode=apiKey" />
  ),
  play: async ({ canvasElement }) => {
    const canvas = within(canvasElement);
    await userEvent.type(canvas.getByLabelText(/display name/i), "Gateway Key");
    await userEvent.type(canvas.getByLabelText(/^api key$/i), "sk-gateway");
    await userEvent.type(canvas.getByLabelText(/upstream base url/i), "proxy.example.com/gateway");
    await expect(
      canvas.getByText(/absolute http\(s\) url|http\(s\) 的绝对 url/i),
    ).toBeInTheDocument();
    await expect(canvas.getByRole("button", { name: /create api key account/i })).toBeDisabled();
  },
};
