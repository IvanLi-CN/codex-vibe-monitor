import type { ForwardProxyBindingNode } from "../lib/api";

export type ForwardProxyBindingOption = ForwardProxyBindingNode & {
  identityHint?: string;
  missing?: boolean;
};

export function normalizeForwardProxyBindingKeys(values?: string[]): string[] {
  if (!Array.isArray(values)) return [];
  return Array.from(
    new Set(
      values.map((value) => value.trim()).filter((value) => value.length > 0),
    ),
  );
}

export function canonicalizeForwardProxyBindingKeys(
  values: string[],
  availableProxyNodes?: ForwardProxyBindingNode[],
): string[] {
  const keyAliases = new Map<string, string>();
  for (const node of Array.isArray(availableProxyNodes)
    ? availableProxyNodes
    : []) {
    keyAliases.set(node.key, node.key);
    for (const alias of normalizeForwardProxyBindingKeys(node.aliasKeys)) {
      keyAliases.set(alias, node.key);
    }
  }
  return Array.from(
    new Set(values.map((value) => keyAliases.get(value) ?? value)),
  );
}

function buildMissingProxyOption(key: string): ForwardProxyBindingOption {
  const isDirect = key === "__direct__";
  return {
    key,
    source: isDirect ? "direct" : "missing",
    displayName: isDirect ? "Direct" : key,
    protocolLabel: isDirect ? "DIRECT" : "UNKNOWN",
    penalized: false,
    selectable: isDirect,
    last24h: [],
    missing: !isDirect,
  };
}

function buildProxyIdentityHint(key: string): string {
  let hash = 0x811c9dc5;
  for (let index = 0; index < key.length; index += 1) {
    hash ^= key.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193);
  }
  return `ID ${(hash >>> 0).toString(36).toUpperCase().slice(-6).padStart(6, "0")}`;
}

function shouldShowProxyIdentityHint(
  node: ForwardProxyBindingOption,
  duplicateDisplayName: boolean,
): boolean {
  if (node.missing || duplicateDisplayName) {
    return true;
  }
  return node.displayName.trim().length > 28;
}

export function resolveForwardProxyBindingOptions(
  selectedKeys: string[],
  availableProxyNodes?: ForwardProxyBindingNode[],
): ForwardProxyBindingOption[] {
  const available = Array.isArray(availableProxyNodes)
    ? availableProxyNodes
        .filter(
          (node) => node.source !== "missing" || selectedKeys.includes(node.key),
        )
        .map((node) => ({
          ...node,
          last24h: Array.isArray(node.last24h) ? node.last24h : [],
        }))
    : [];
  const availableByKey = new Map(available.map((node) => [node.key, node]));
  const options: ForwardProxyBindingOption[] = [...available];
  for (const key of selectedKeys) {
    if (!availableByKey.has(key)) {
      options.push(buildMissingProxyOption(key));
    }
  }
  const displayNameCounts = new Map<string, number>();
  for (const node of options) {
    const normalizedDisplayName = node.displayName.trim();
    displayNameCounts.set(
      normalizedDisplayName,
      (displayNameCounts.get(normalizedDisplayName) ?? 0) + 1,
    );
  }
  return options.map((node) => {
    const duplicateDisplayName =
      (displayNameCounts.get(node.displayName.trim()) ?? 0) > 1;
    return {
      ...node,
      identityHint: shouldShowProxyIdentityHint(node, duplicateDisplayName)
        ? buildProxyIdentityHint(node.key)
        : undefined,
    };
  });
}

export function hasSelectableForwardProxyBindingSelection(
  selectedKeys: string[],
  options: ForwardProxyBindingOption[],
): boolean {
  return (
    selectedKeys.length === 0 ||
    selectedKeys.some((key) =>
      options.some((node) => node.key === key && node.selectable),
    )
  );
}
