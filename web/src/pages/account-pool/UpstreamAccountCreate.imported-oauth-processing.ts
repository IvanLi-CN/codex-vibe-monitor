// biome-ignore-all lint/correctness/useExhaustiveDependencies: synchronization effects deliberately depend on mutable refs and preserve request ordering

import type { ChangeEvent, ClipboardEvent } from "react";
import { useCallback, useEffect } from "react";
import type { ImportOauthCredentialFilePayload } from "../../lib/api";
import type { UpstreamAccountCreateControllerContext } from "./UpstreamAccountCreate.controller-context";
import type { useImportedOauthValidation } from "./UpstreamAccountCreate.imported-oauth-validation";
import {
  convertImportedWebSessionDocumentLocally,
  createImportedOauthPastedFileName,
  createImportedOauthPastedSourceId,
  createImportedOauthSourceId,
  createImportedSessionPastedFileName,
  type ParsedImportedOauthCredentialRejection,
  parseImportedOauthCredentialDocumentLocally,
  parseImportedOauthPasteDraft,
  validateImportedOauthCredentialLocally,
} from "./UpstreamAccountCreate.shared";

type LocalImportedOauthCandidate = {
  fileName: string;
  payload: ImportOauthCredentialFilePayload;
  matchKey: string;
};

type LocalImportedOauthRejection = {
  fileName: string;
  reason: string;
  duplicate?: boolean;
  warning?: boolean;
};

type LocalImportedOauthCandidatesResult =
  | {
      ok: true;
      candidates: LocalImportedOauthCandidate[];
      rejected: LocalImportedOauthRejection[];
    }
  | {
      ok: false;
      error: string;
      rejected: LocalImportedOauthRejection[];
    };

type LocalImportedSessionCandidatesResult = LocalImportedOauthCandidatesResult;
type ValidationController = ReturnType<typeof useImportedOauthValidation>;

function importedOauthErrorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function summarizeRejectedImportedOauthItems(
  rejected: LocalImportedOauthRejection[],
  t: UpstreamAccountCreateControllerContext["t"],
  setImportSelectionFeedback: UpstreamAccountCreateControllerContext["setImportSelectionFeedback"],
): void {
  if (rejected.length === 0) {
    setImportSelectionFeedback(null);
    return;
  }
  setImportSelectionFeedback({
    variant: rejected.some((item) => item.duplicate !== true && item.warning !== true)
      ? "error"
      : "warning",
    messages: rejected.map((item) =>
      item.duplicate
        ? t("accountPool.upstreamAccounts.import.local.duplicateSkipped", {
            fileName: item.fileName,
          })
        : t("accountPool.upstreamAccounts.import.local.fileRejected", {
            fileName: item.fileName,
            reason: item.reason,
          }),
    ),
  });
}

function collectQueuedImportedOauthMatchKeys(
  importFilesRef: UpstreamAccountCreateControllerContext["importFilesRef"],
  t: UpstreamAccountCreateControllerContext["t"],
): Set<string> {
  const matchKeys = new Set<string>();
  for (const item of importFilesRef.current as ImportOauthCredentialFilePayload[]) {
    const parsed = validateImportedOauthCredentialLocally(item.content, t);
    if (parsed.ok && parsed.matchKey) matchKeys.add(parsed.matchKey);
  }
  return matchKeys;
}

function buildWebSessionCandidates({
  content,
  fileName,
  createSourceId,
  t,
}: {
  content: string;
  fileName: string;
  createSourceId: (index: number) => string;
  t: UpstreamAccountCreateControllerContext["t"];
}): LocalImportedSessionCandidatesResult {
  const parsed = convertImportedWebSessionDocumentLocally(content, t);
  if (!parsed.ok) return { ok: false, error: parsed.error, rejected: [] };
  return {
    ok: true,
    candidates: parsed.items.map((item, index) => {
      const nextFileName =
        parsed.items.length === 1
          ? fileName
          : `${fileName.replace(/\.json$/i, "")} session ${index + 1}.json`;
      return {
        fileName: nextFileName,
        matchKey: item.matchKey,
        payload: { sourceId: createSourceId(index), fileName: nextFileName, content: item.content },
      };
    }),
    rejected: [],
  };
}

function buildExpandedImportedOauthFileName({
  baseFileName,
  sourceLabel,
  total,
  index,
}: {
  baseFileName: string;
  sourceLabel: string;
  total: number;
  index: number;
}): string {
  if (total <= 1) return baseFileName;
  const normalizedSourceLabel = sourceLabel.trim() || `account ${index + 1}`;
  return `${baseFileName.replace(/\.json$/i, "")} ${normalizedSourceLabel}.json`;
}

function mapImportedOauthRejections(
  rejected: ParsedImportedOauthCredentialRejection[],
  fileName: string,
  total: number,
): LocalImportedOauthRejection[] {
  return rejected.map((item, index) => ({
    fileName: buildExpandedImportedOauthFileName({
      baseFileName: fileName,
      sourceLabel: item.sourceLabel,
      total,
      index,
    }),
    reason: item.reason,
    warning: true,
  }));
}

function buildImportedOauthCandidates({
  content,
  fileName,
  createSourceId,
  t,
}: {
  content: string;
  fileName: string;
  createSourceId: (index: number) => string;
  t: UpstreamAccountCreateControllerContext["t"];
}): LocalImportedOauthCandidatesResult {
  const parsed = parseImportedOauthCredentialDocumentLocally(content, t);
  if (!parsed.ok) {
    return {
      ok: false,
      error: parsed.error,
      rejected: mapImportedOauthRejections(parsed.rejected, fileName, parsed.rejected.length),
    };
  }
  const totalEntries = parsed.candidates.length + parsed.rejected.length;
  return {
    ok: true,
    candidates: parsed.candidates.map((candidate, index) => {
      const nextFileName = buildExpandedImportedOauthFileName({
        baseFileName: fileName,
        sourceLabel: candidate.sourceLabel,
        total: totalEntries,
        index,
      });
      return {
        fileName: nextFileName,
        matchKey: candidate.matchKey,
        payload: {
          sourceId: totalEntries <= 1 ? createSourceId(0) : createSourceId(index),
          fileName: nextFileName,
          content: candidate.normalizedContent,
        },
      };
    }),
    rejected: mapImportedOauthRejections(parsed.rejected, fileName, totalEntries),
  };
}

function parseImportedOauthPaste({
  content,
  isImportingWebSession,
  serial,
  t,
}: {
  content: string;
  isImportingWebSession: boolean;
  serial: number;
  t: UpstreamAccountCreateControllerContext["t"];
}) {
  const pastedFileName = isImportingWebSession
    ? createImportedSessionPastedFileName(serial)
    : createImportedOauthPastedFileName(serial);
  if (isImportingWebSession) {
    return {
      parsedDraft: null,
      parsedSessionDraft: buildWebSessionCandidates({
        content,
        fileName: pastedFileName,
        createSourceId: (index) => `${createImportedOauthPastedSourceId(serial)}:session:${index}`,
        t,
      }),
    };
  }
  return { parsedDraft: parseImportedOauthPasteDraft(content, t), parsedSessionDraft: null };
}

function buildPastedImportedOauthItems({
  serial,
  parsedDraft,
  parsedSessionDraft,
}: {
  serial: number;
  parsedDraft: ReturnType<typeof parseImportedOauthPasteDraft> | null;
  parsedSessionDraft: LocalImportedSessionCandidatesResult | null;
}): {
  items: Array<ImportOauthCredentialFilePayload & { matchKey: string }>;
  rejectedItems: LocalImportedOauthRejection[];
} {
  const items: Array<ImportOauthCredentialFilePayload & { matchKey: string }> = [];
  const rejectedItems: LocalImportedOauthRejection[] = [];
  if (parsedSessionDraft?.ok) {
    items.push(
      ...parsedSessionDraft.candidates.map((candidate, index) => ({
        ...candidate.payload,
        sourceId: `${createImportedOauthPastedSourceId(serial)}:session:${index}`,
        fileName: createImportedSessionPastedFileName(
          serial,
          parsedSessionDraft.candidates.length === 1 ? undefined : index,
        ),
        matchKey: candidate.matchKey,
      })),
    );
    return { items, rejectedItems };
  }
  if (!parsedDraft?.ok) return { items, rejectedItems };
  const total = parsedDraft.candidates.length + parsedDraft.rejected.length;
  rejectedItems.push(
    ...parsedDraft.rejected.map((item) => ({
      fileName:
        total <= 1
          ? createImportedOauthPastedFileName(serial)
          : `${createImportedOauthPastedFileName(serial).replace(/\.json$/i, "")} ${item.sourceLabel}.json`,
      reason: item.reason,
      duplicate: false,
      warning: true,
    })),
  );
  items.push(
    ...parsedDraft.candidates.map((candidate, index) => ({
      sourceId:
        total <= 1
          ? createImportedOauthPastedSourceId(serial)
          : `${createImportedOauthPastedSourceId(serial)}:oauth:${index}`,
      fileName:
        total <= 1
          ? createImportedOauthPastedFileName(serial)
          : `${createImportedOauthPastedFileName(serial).replace(/\.json$/i, "")} ${candidate.sourceLabel}.json`,
      content: candidate.normalizedContent,
      matchKey: candidate.matchKey,
    })),
  );
  return { items, rejectedItems };
}

function filterDuplicateImportedOauthItems<T extends { matchKey: string; fileName: string }>(
  items: T[],
  seenKeys: Set<string>,
  rejectedItems: LocalImportedOauthRejection[],
): T[] {
  const accepted: T[] = [];
  for (const item of items) {
    if (seenKeys.has(item.matchKey)) {
      rejectedItems.push({ fileName: item.fileName, reason: "", duplicate: true });
      continue;
    }
    seenKeys.add(item.matchKey);
    accepted.push(item);
  }
  return accepted;
}

function reportEmptyPastedImport({
  rejectedItems,
  items,
  t,
  setImportPasteError,
  summarizeRejected,
}: {
  rejectedItems: LocalImportedOauthRejection[];
  items: unknown[];
  t: UpstreamAccountCreateControllerContext["t"];
  setImportPasteError: UpstreamAccountCreateControllerContext["setImportPasteError"];
  summarizeRejected: (items: LocalImportedOauthRejection[]) => void;
}): void {
  if (rejectedItems.length === 1 && rejectedItems[0]?.duplicate === true && items.length === 1) {
    setImportPasteError(t("accountPool.upstreamAccounts.import.local.pasteDuplicate"));
    return;
  }
  summarizeRejected(rejectedItems);
  setImportPasteError(
    rejectedItems[0]?.duplicate
      ? t("accountPool.upstreamAccounts.import.local.pasteDuplicate")
      : (rejectedItems[0]?.reason ?? null),
  );
}

type QueueImportedOauthPasteOptions = {
  draftContent: string;
  requestedSerial?: number | null;
  isImportingWebSession: boolean;
  refs: Pick<
    UpstreamAccountCreateControllerContext,
    | "importPasteSequenceRef"
    | "importPasteValidationTokenRef"
    | "importPasteDraftRef"
    | "importFilesRef"
    | "importFilesRevisionRef"
  >;
  setters: Pick<
    UpstreamAccountCreateControllerContext,
    | "setImportPasteDraftSerial"
    | "setImportPasteBusy"
    | "setImportPasteError"
    | "setActionError"
    | "setImportSelectionFeedback"
    | "setImportFiles"
    | "setImportPasteDraft"
  >;
  t: UpstreamAccountCreateControllerContext["t"];
  resetImportValidationForSelectionChange: ValidationController["resetImportValidationForSelectionChange"];
  collectQueuedMatchKeys: () => Set<string>;
  summarizeRejected: (items: LocalImportedOauthRejection[]) => void;
};

async function commitImportedOauthPaste({
  normalizedDraft,
  validationToken,
  built,
  refs,
  setters,
  t,
  resetImportValidationForSelectionChange,
  collectQueuedMatchKeys,
  summarizeRejected,
}: Omit<
  QueueImportedOauthPasteOptions,
  "draftContent" | "requestedSerial" | "isImportingWebSession"
> & {
  normalizedDraft: string;
  validationToken: number;
  built: ReturnType<typeof buildPastedImportedOauthItems>;
}): Promise<void> {
  try {
    const initiallyAcceptedItems = filterDuplicateImportedOauthItems(
      built.items,
      collectQueuedMatchKeys(),
      built.rejectedItems,
    );
    if (initiallyAcceptedItems.length === 0) {
      reportEmptyPastedImport({
        rejectedItems: built.rejectedItems,
        items: built.items,
        t,
        setImportPasteError: setters.setImportPasteError,
        summarizeRejected,
      });
      return;
    }
    await resetImportValidationForSelectionChange();
    if (
      validationToken !== refs.importPasteValidationTokenRef.current ||
      refs.importPasteDraftRef.current.trim() !== normalizedDraft
    ) {
      return;
    }
    const acceptedItems = filterDuplicateImportedOauthItems(
      initiallyAcceptedItems,
      collectQueuedMatchKeys(),
      built.rejectedItems,
    ).map(({ matchKey: _matchKey, ...item }) => item);
    if (acceptedItems.length === 0) {
      reportEmptyPastedImport({
        rejectedItems: built.rejectedItems,
        items: built.items,
        t,
        setImportPasteError: setters.setImportPasteError,
        summarizeRejected,
      });
      return;
    }
    refs.importFilesRevisionRef.current += 1;
    const nextItems = [
      ...(refs.importFilesRef.current as ImportOauthCredentialFilePayload[]),
      ...acceptedItems,
    ];
    refs.importFilesRef.current = nextItems;
    setters.setImportFiles(nextItems);
    refs.importPasteDraftRef.current = "";
    setters.setImportPasteDraft("");
    setters.setImportPasteDraftSerial(null);
    setters.setImportPasteError(null);
    summarizeRejected(built.rejectedItems);
  } catch (error) {
    if (validationToken === refs.importPasteValidationTokenRef.current) {
      setters.setImportPasteError(importedOauthErrorMessage(error));
    }
  } finally {
    if (validationToken === refs.importPasteValidationTokenRef.current) {
      setters.setImportPasteBusy(false);
    }
  }
}

async function queueImportedOauthPaste({
  draftContent,
  requestedSerial,
  isImportingWebSession,
  refs,
  setters,
  t,
  resetImportValidationForSelectionChange,
  collectQueuedMatchKeys,
  summarizeRejected,
}: QueueImportedOauthPasteOptions): Promise<void> {
  const normalizedDraft = draftContent.trim();
  const serial =
    requestedSerial && requestedSerial > 0
      ? requestedSerial
      : refs.importPasteSequenceRef.current + 1;
  const parsed = parseImportedOauthPaste({
    content: draftContent,
    isImportingWebSession,
    serial,
    t,
  });
  if (parsed.parsedDraft && !parsed.parsedDraft.ok) {
    setters.setImportPasteError(parsed.parsedDraft.error);
    return;
  }
  if (parsed.parsedSessionDraft && !parsed.parsedSessionDraft.ok) {
    setters.setImportPasteError(parsed.parsedSessionDraft.error);
    return;
  }
  if (!requestedSerial || requestedSerial <= 0) {
    refs.importPasteSequenceRef.current = serial;
  }
  const validationToken = refs.importPasteValidationTokenRef.current + 1;
  refs.importPasteValidationTokenRef.current = validationToken;
  setters.setImportPasteDraftSerial(serial);
  setters.setImportPasteBusy(true);
  setters.setImportPasteError(null);
  setters.setActionError(null);
  setters.setImportSelectionFeedback(null);
  const built = buildPastedImportedOauthItems({
    serial,
    parsedDraft: parsed.parsedDraft,
    parsedSessionDraft: parsed.parsedSessionDraft,
  });
  if (built.items.length === 0) {
    setters.setImportPasteError(
      built.rejectedItems[0]?.reason ??
        t("accountPool.upstreamAccounts.import.local.noSupportedSub2apiAccounts"),
    );
    return;
  }
  await commitImportedOauthPaste({
    normalizedDraft,
    validationToken,
    built,
    refs,
    setters,
    t,
    resetImportValidationForSelectionChange,
    collectQueuedMatchKeys,
    summarizeRejected,
  });
}

function createImportedOauthProcessingHelpers(ctx: UpstreamAccountCreateControllerContext) {
  const { importFilesRef, t, setImportSelectionFeedback } = ctx;
  return {
    summarizeRejected: (items: LocalImportedOauthRejection[]) =>
      summarizeRejectedImportedOauthItems(items, t, setImportSelectionFeedback),
    collectQueuedMatchKeys: () => collectQueuedImportedOauthMatchKeys(importFilesRef, t),
    buildWebCandidates: (options: Omit<Parameters<typeof buildWebSessionCandidates>[0], "t">) =>
      buildWebSessionCandidates({ ...options, t }),
    buildOauthCandidates: (
      options: Omit<Parameters<typeof buildImportedOauthCandidates>[0], "t">,
    ) => buildImportedOauthCandidates({ ...options, t }),
  };
}

function useImportedOauthPasteHandlers(
  ctx: UpstreamAccountCreateControllerContext,
  validation: ValidationController,
  helpers: ReturnType<typeof createImportedOauthProcessingHelpers>,
) {
  const {
    activeTab,
    importPasteDraft,
    importPasteDraftRef,
    importPasteDraftSerial,
    importPasteSequenceRef,
    importPasteValidationTokenRef,
    importFilesRef,
    importFilesRevisionRef,
    setActionError,
    setImportPasteDraft,
    setImportPasteDraftSerial,
    setImportPasteBusy,
    setImportPasteError,
    setImportSelectionFeedback,
    setImportFiles,
    t,
    writesEnabled,
  } = ctx;
  const isImportingWebSession = activeTab === "importSession";
  const validateAndQueueImportedOauthPaste = useCallback(
    async (draftContent: string, options?: { serial?: number | null }) =>
      queueImportedOauthPaste({
        draftContent,
        requestedSerial: options?.serial,
        isImportingWebSession,
        refs: {
          importPasteSequenceRef,
          importPasteValidationTokenRef,
          importPasteDraftRef,
          importFilesRef,
          importFilesRevisionRef,
        },
        setters: {
          setImportPasteDraftSerial,
          setImportPasteBusy,
          setImportPasteError,
          setActionError,
          setImportSelectionFeedback,
          setImportFiles,
          setImportPasteDraft,
        },
        t,
        resetImportValidationForSelectionChange: validation.resetImportValidationForSelectionChange,
        collectQueuedMatchKeys: helpers.collectQueuedMatchKeys,
        summarizeRejected: helpers.summarizeRejected,
      }),
    [helpers, isImportingWebSession, t, validation.resetImportValidationForSelectionChange],
  );
  const handleImportedOauthPasteDraftChange = useCallback(
    (event: ChangeEvent<HTMLTextAreaElement>) => {
      importPasteValidationTokenRef.current += 1;
      importPasteDraftRef.current = event.target.value;
      setImportPasteDraft(event.target.value);
      setImportPasteBusy(false);
      setImportPasteError(null);
      setActionError(null);
    },
    [],
  );
  const handleImportedOauthPaste = useCallback(
    (event: ClipboardEvent<HTMLTextAreaElement>) => {
      event.preventDefault();
      const nextDraft =
        event.clipboardData.getData("text/plain") || event.clipboardData.getData("text");
      importPasteValidationTokenRef.current += 1;
      importPasteSequenceRef.current += 1;
      const serial = importPasteSequenceRef.current;
      importPasteDraftRef.current = nextDraft;
      setImportPasteDraft(nextDraft);
      setImportPasteDraftSerial(serial);
      setImportPasteBusy(false);
      setImportPasteError(null);
      setActionError(null);
      setImportSelectionFeedback(null);
      void validateAndQueueImportedOauthPaste(nextDraft, { serial });
    },
    [validateAndQueueImportedOauthPaste],
  );
  const handleValidateImportedOauthPasteDraft = useCallback(async () => {
    if (!writesEnabled) return;
    await validateAndQueueImportedOauthPaste(importPasteDraft, { serial: importPasteDraftSerial });
  }, [importPasteDraft, importPasteDraftSerial, validateAndQueueImportedOauthPaste, writesEnabled]);
  useEffect(() => {
    importPasteDraftRef.current = importPasteDraft;
  }, [importPasteDraft]);
  return {
    validateAndQueueImportedOauthPaste,
    handleImportedOauthPasteDraftChange,
    handleImportedOauthPaste,
    handleValidateImportedOauthPasteDraft,
  };
}

async function parseSelectedImportedOauthFiles({
  selectedFiles,
  sourceIdOffset,
  isImportingWebSession,
  buildWebCandidates,
  buildOauthCandidates,
}: {
  selectedFiles: File[];
  sourceIdOffset: number;
  isImportingWebSession: boolean;
  buildWebCandidates: ReturnType<typeof createImportedOauthProcessingHelpers>["buildWebCandidates"];
  buildOauthCandidates: ReturnType<
    typeof createImportedOauthProcessingHelpers
  >["buildOauthCandidates"];
}) {
  return Promise.all(
    selectedFiles.map(async (file, index) => {
      const content = await file.text();
      const sourceIdBase = createImportedOauthSourceId(file, sourceIdOffset + index);
      const parsed = isImportingWebSession
        ? buildWebCandidates({
            content,
            fileName: file.name,
            createSourceId: (sessionIndex) => `${sourceIdBase}:session:${sessionIndex}`,
          })
        : buildOauthCandidates({
            content,
            fileName: file.name,
            createSourceId: (oauthIndex) =>
              oauthIndex === 0 ? sourceIdBase : `${sourceIdBase}:oauth:${oauthIndex}`,
          });
      return { fileName: file.name, parsed };
    }),
  );
}

function collectImportedOauthFileCandidates(
  parsedItems: Awaited<ReturnType<typeof parseSelectedImportedOauthFiles>>,
): {
  validCandidates: LocalImportedOauthCandidate[];
  rejectedItems: LocalImportedOauthRejection[];
} {
  const validCandidates: LocalImportedOauthCandidate[] = [];
  const rejectedItems: LocalImportedOauthRejection[] = [];
  for (const item of parsedItems) {
    if (!item.parsed.ok) {
      rejectedItems.push(
        ...(item.parsed.rejected.length > 0
          ? item.parsed.rejected
          : [{ fileName: item.fileName, reason: item.parsed.error }]),
      );
      continue;
    }
    rejectedItems.push(...item.parsed.rejected);
    validCandidates.push(...item.parsed.candidates);
  }
  return { validCandidates, rejectedItems };
}

async function appendImportedOauthFileCandidates({
  acceptedCandidates,
  rejectedItems,
  ctx,
  resetImportValidationForSelectionChange,
  collectQueuedMatchKeys,
}: {
  acceptedCandidates: LocalImportedOauthCandidate[];
  rejectedItems: LocalImportedOauthRejection[];
  ctx: UpstreamAccountCreateControllerContext;
  resetImportValidationForSelectionChange: ValidationController["resetImportValidationForSelectionChange"];
  collectQueuedMatchKeys: () => Set<string>;
}): Promise<boolean> {
  if (acceptedCandidates.length === 0) return false;
  await resetImportValidationForSelectionChange();
  const seenKeys = collectQueuedMatchKeys();
  const acceptedItems = acceptedCandidates.filter((candidate) => {
    if (seenKeys.has(candidate.matchKey)) {
      rejectedItems.push({ fileName: candidate.fileName, reason: "", duplicate: true });
      return false;
    }
    seenKeys.add(candidate.matchKey);
    return true;
  });
  if (acceptedItems.length === 0) return false;
  const { importFilesRef, importFilesRevisionRef, setImportFiles } = ctx;
  importFilesRevisionRef.current += 1;
  const nextItems = [
    ...(importFilesRef.current as ImportOauthCredentialFilePayload[]),
    ...acceptedItems.map((candidate) => candidate.payload),
  ];
  importFilesRef.current = nextItems;
  setImportFiles(nextItems);
  return true;
}

function useImportedOauthFileHandlers(
  ctx: UpstreamAccountCreateControllerContext,
  validation: ValidationController,
  helpers: ReturnType<typeof createImportedOauthProcessingHelpers>,
) {
  const {
    activeTab,
    importFileSourceSequenceRef,
    importFilesRef,
    importPasteValidationTokenRef,
    setActionError,
    setImportPasteBusy,
    setImportInputKey,
    setImportFiles,
  } = ctx;
  const isImportingWebSession = activeTab === "importSession";
  const handleImportFilesChange = useCallback(
    async (event: ChangeEvent<HTMLInputElement>) => {
      const selectedFiles = Array.from(event.target.files ?? []);
      setActionError(null);
      if (selectedFiles.length === 0) return;
      try {
        importPasteValidationTokenRef.current += 1;
        setImportPasteBusy(false);
        const sourceIdOffset = importFileSourceSequenceRef.current;
        importFileSourceSequenceRef.current += selectedFiles.length;
        const parsedItems = await parseSelectedImportedOauthFiles({
          selectedFiles,
          sourceIdOffset,
          isImportingWebSession,
          buildWebCandidates: helpers.buildWebCandidates,
          buildOauthCandidates: helpers.buildOauthCandidates,
        });
        const { validCandidates, rejectedItems } = collectImportedOauthFileCandidates(parsedItems);
        const acceptedCandidates = filterDuplicateImportedOauthItems(
          validCandidates,
          helpers.collectQueuedMatchKeys(),
          rejectedItems,
        );
        await appendImportedOauthFileCandidates({
          acceptedCandidates,
          rejectedItems,
          ctx,
          resetImportValidationForSelectionChange:
            validation.resetImportValidationForSelectionChange,
          collectQueuedMatchKeys: helpers.collectQueuedMatchKeys,
        });
        helpers.summarizeRejected(rejectedItems);
        setImportInputKey((current: number) => current + 1);
      } catch (error) {
        setActionError(importedOauthErrorMessage(error));
      }
    },
    [helpers, isImportingWebSession, validation.resetImportValidationForSelectionChange],
  );
  const handleClearImportSelection = useCallback(() => {
    void (async () => {
      importPasteValidationTokenRef.current += 1;
      setImportPasteBusy(false);
      await validation.resetImportValidationForSelectionChange();
      ctx.importFilesRevisionRef.current += 1;
      importFilesRef.current = [];
      setImportFiles([]);
      ctx.setImportSelectionFeedback(null);
      setImportInputKey((current: number) => current + 1);
    })();
  }, [validation.resetImportValidationForSelectionChange]);
  return { handleImportFilesChange, handleClearImportSelection };
}

export function useImportedOauthProcessing(
  ctx: UpstreamAccountCreateControllerContext,
  validation: ValidationController,
) {
  const helpers = createImportedOauthProcessingHelpers(ctx);
  const paste = useImportedOauthPasteHandlers(ctx, validation, helpers);
  const files = useImportedOauthFileHandlers(ctx, validation, helpers);
  return { ...paste, ...files };
}
