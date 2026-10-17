import type {
  ModelsDevPriceCandidate,
  ModelsDevSyncMemoryState,
  PricingEntry,
  PricingQuote,
} from "../../../lib/api";

export const PRICE_FIELDS = [
  ["inputPer1m", "input"],
  ["outputPer1m", "output"],
  ["cacheReadPer1m", "cacheRead"],
  ["cacheWritePer1m", "cacheWrite"],
  ["reasoningPer1m", "reasoning"],
] as const;

export type PriceField = (typeof PRICE_FIELDS)[number][0];

export interface ModelCandidateGroup {
  model: string;
  candidates: ModelsDevPriceCandidate[];
}

export function modelProviderKey(model: string, providerId: string): string {
  return JSON.stringify([model, providerId]);
}

export function buildModelCandidateGroups(
  candidates: ModelsDevPriceCandidate[],
  selectedProviders: ReadonlySet<string>,
  showDeprecated: boolean,
): ModelCandidateGroup[] {
  const groups = new Map<string, ModelsDevPriceCandidate[]>();
  for (const candidate of candidates) {
    if (!selectedProviders.has(candidate.providerId)) continue;
    if (!showDeprecated && candidate.status === "deprecated") continue;
    const group = groups.get(candidate.model) ?? [];
    group.push(candidate);
    groups.set(candidate.model, group);
  }
  return Array.from(groups, ([model, groupedCandidates]) => ({
    model,
    candidates: groupedCandidates,
  })).sort((left, right) => left.model.localeCompare(right.model));
}

export function filterModelCandidateGroups(
  groups: ModelCandidateGroup[],
  search: string,
): ModelCandidateGroup[] {
  const query = search.trim().toLocaleLowerCase();
  if (!query) return groups;
  return groups.filter(
    ({ model, candidates }) =>
      model.toLocaleLowerCase().includes(query) ||
      candidates.some((candidate) => candidate.name.toLocaleLowerCase().includes(query)),
  );
}

export function resolveModelCandidate(
  group: ModelCandidateGroup,
  quoteProviderId: string | undefined,
): ModelsDevPriceCandidate | undefined {
  if (quoteProviderId !== undefined) {
    return group.candidates.find((candidate) => candidate.providerId === quoteProviderId);
  }
  return group.candidates.length === 1 ? group.candidates[0] : undefined;
}

export function localPriceFields(entry: PricingEntry | undefined) {
  return {
    inputPer1m: entry?.inputPer1m ?? null,
    outputPer1m: entry?.outputPer1m ?? null,
    cacheReadPer1m: entry?.cacheReadPer1m ?? entry?.cacheInputPer1m ?? null,
    cacheWritePer1m: entry?.cacheWritePer1m ?? null,
    reasoningPer1m: entry?.reasoningPer1m ?? null,
  };
}

export function candidatePriceFields(candidate: ModelsDevPriceCandidate) {
  return {
    inputPer1m: candidate.inputPer1m,
    outputPer1m: candidate.outputPer1m,
    cacheReadPer1m: candidate.cacheReadPer1m,
    cacheWritePer1m: candidate.cacheWritePer1m,
    reasoningPer1m: candidate.reasoningPer1m,
  };
}

export function priceDifferenceFields(
  local: PricingEntry | undefined,
  candidate: ModelsDevPriceCandidate,
): Set<PriceField> {
  if (!local) return new Set();
  const existing = localPriceFields(local);
  const incoming = candidatePriceFields(candidate);
  return new Set(
    PRICE_FIELDS.flatMap(([field]) => (existing[field] !== incoming[field] ? [field] : [])),
  );
}

export function modelPricesEqual(
  local: PricingEntry | undefined,
  candidate: ModelsDevPriceCandidate,
): boolean {
  return local !== undefined && priceDifferenceFields(local, candidate).size === 0;
}

export function selectionMemoryByKey(memory: ModelsDevSyncMemoryState): Record<string, boolean> {
  return Object.fromEntries(
    memory.modelSelections.map(({ model, providerId, selected }) => [
      modelProviderKey(model, providerId),
      selected,
    ]),
  );
}

export function quoteProviderChoicesByModel(
  memory: ModelsDevSyncMemoryState,
): Record<string, string> {
  return Object.fromEntries(
    memory.quoteProviderChoices.map(({ model, providerId }) => [model, providerId]),
  );
}

export function createApplicablePriceEntries(
  groups: ModelCandidateGroup[],
  pricesByModel: ReadonlyMap<string, PricingEntry>,
  selectionsByKey: Readonly<Record<string, boolean>>,
  quoteChoicesByModel: Readonly<Record<string, string>>,
): PricingEntry[] {
  const entries: PricingEntry[] = [];
  for (const group of groups) {
    const candidate = resolveModelCandidate(group, quoteChoicesByModel[group.model]);
    if (!candidate?.importable) continue;
    if (!selectionsByKey[modelProviderKey(group.model, candidate.providerId)]) continue;
    if (modelPricesEqual(pricesByModel.get(group.model), candidate)) continue;
    entries.push({
      model: candidate.model,
      inputPer1m: candidate.inputPer1m ?? 0,
      outputPer1m: candidate.outputPer1m ?? 0,
      cacheInputPer1m: candidate.cacheReadPer1m,
      cacheReadPer1m: candidate.cacheReadPer1m,
      cacheWritePer1m: candidate.cacheWritePer1m,
      reasoningPer1m: candidate.reasoningPer1m,
      source: "litellm",
    });
  }
  return entries;
}

export function quoteForCandidate(
  quotes: readonly PricingQuote[],
  model: string,
  providerId: string,
): PricingQuote | undefined {
  return quotes.find((quote) => quote.model === model && quote.providerId === providerId);
}

export function quoteContentEqual(left: PricingQuote, right: PricingQuote): boolean {
  if (
    left.model !== right.model ||
    left.providerId !== right.providerId ||
    left.providerName !== right.providerName ||
    left.source !== right.source ||
    left.status !== right.status ||
    left.deprecationDate !== right.deprecationDate ||
    left.sourceUrl !== right.sourceUrl ||
    left.rates.length !== right.rates.length
  ) {
    return false;
  }
  return left.rates.every((rate, index) => {
    const other = right.rates[index];
    return (
      other !== undefined &&
      rate.key === other.key &&
      rate.sourceValue === other.sourceValue &&
      rate.sourceUnit === other.sourceUnit &&
      rate.value === other.value &&
      rate.unit === other.unit &&
      rate.dimension === other.dimension &&
      rate.modality === other.modality &&
      rate.tier === other.tier &&
      rate.threshold === other.threshold
    );
  });
}

export function createApplicableQuotes(
  groups: ModelCandidateGroup[],
  quotes: readonly PricingQuote[],
  existingQuotesByModel: ReadonlyMap<string, readonly PricingQuote[]>,
  selectionsByKey: Readonly<Record<string, boolean>>,
  quoteChoicesByModel: Readonly<Record<string, string>>,
): PricingQuote[] {
  const selectedQuotes: PricingQuote[] = [];
  for (const group of groups) {
    const candidate = resolveModelCandidate(group, quoteChoicesByModel[group.model]);
    if (!candidate || !selectionsByKey[modelProviderKey(group.model, candidate.providerId)]) {
      continue;
    }
    const quote = quoteForCandidate(quotes, group.model, candidate.providerId);
    const existingQuote = existingQuotesByModel
      .get(group.model)
      ?.find((item) => item.providerId === candidate.providerId);
    if (quote && (!existingQuote || !quoteContentEqual(existingQuote, quote))) {
      selectedQuotes.push(quote);
    }
  }
  return selectedQuotes;
}

export function applySelectionChanges(
  current: Readonly<Record<string, boolean>>,
  rows: Array<{ model: string; providerId: string }>,
  value: boolean | "invert",
): Record<string, boolean> {
  const next = { ...current };
  for (const row of rows) {
    const key = modelProviderKey(row.model, row.providerId);
    next[key] = value === "invert" ? !current[key] : value;
  }
  return next;
}
