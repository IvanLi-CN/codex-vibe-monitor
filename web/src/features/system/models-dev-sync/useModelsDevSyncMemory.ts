import { useCallback, useRef, useState } from "react";
import {
  type ModelsDevSyncMemoryPatch,
  type ModelsDevSyncMemoryState,
  updateModelsDevSyncMemory,
} from "../../../lib/api";
import { modelProviderKey } from "./selection";

const EMPTY_MEMORY: ModelsDevSyncMemoryState = {
  catalogBaselineInitialized: false,
  providerSelectionInitialized: false,
  providerSelections: [],
  modelSelections: [],
  quoteProviderChoices: [],
  unviewedModelIds: [],
};

function emptyPatch(): ModelsDevSyncMemoryPatch {
  return {
    providerSelections: [],
    modelSelections: [],
    quoteProviderChoices: [],
    viewedModelIds: [],
  };
}

function mergePatches(
  first: ModelsDevSyncMemoryPatch,
  second: ModelsDevSyncMemoryPatch,
): ModelsDevSyncMemoryPatch {
  const providers = new Map(
    (first.providerSelections ?? []).map((change) => [change.providerId, change]),
  );
  second.providerSelections?.forEach((change) => {
    providers.set(change.providerId, change);
  });
  const selections = new Map(
    (first.modelSelections ?? []).map((change) => [
      modelProviderKey(change.model, change.providerId),
      change,
    ]),
  );
  second.modelSelections?.forEach((change) => {
    selections.set(modelProviderKey(change.model, change.providerId), change);
  });
  const quoteChoices = new Map(
    (first.quoteProviderChoices ?? []).map((change) => [change.model, change]),
  );
  second.quoteProviderChoices?.forEach((change) => {
    quoteChoices.set(change.model, change);
  });
  return {
    providerSelections: Array.from(providers.values()),
    modelSelections: Array.from(selections.values()),
    quoteProviderChoices: Array.from(quoteChoices.values()),
    viewedModelIds: Array.from(
      new Set([...(first.viewedModelIds ?? []), ...(second.viewedModelIds ?? [])]),
    ),
  };
}

function patchMemory(
  current: ModelsDevSyncMemoryState,
  patch: ModelsDevSyncMemoryPatch,
): ModelsDevSyncMemoryState {
  const providers = new Map(
    current.providerSelections.map((selection) => [selection.providerId, selection]),
  );
  patch.providerSelections?.forEach((selection) => {
    providers.set(selection.providerId, selection);
  });
  const selections = new Map(
    current.modelSelections.map((selection) => [
      modelProviderKey(selection.model, selection.providerId),
      selection,
    ]),
  );
  patch.modelSelections?.forEach((selection) => {
    selections.set(modelProviderKey(selection.model, selection.providerId), selection);
  });
  const quoteChoices = new Map(
    current.quoteProviderChoices.map((choice) => [choice.model, choice]),
  );
  patch.quoteProviderChoices?.forEach((choice) => {
    quoteChoices.set(choice.model, choice);
  });
  const viewed = new Set(patch.viewedModelIds ?? []);
  return {
    ...current,
    providerSelections: Array.from(providers.values()),
    modelSelections: Array.from(selections.values()),
    quoteProviderChoices: Array.from(quoteChoices.values()),
    unviewedModelIds: current.unviewedModelIds.filter((model) => !viewed.has(model)),
  };
}

function overlayUnsavedMemory(
  state: ModelsDevSyncMemoryState,
  inflight: ModelsDevSyncMemoryPatch,
  pending: ModelsDevSyncMemoryPatch,
): ModelsDevSyncMemoryState {
  return patchMemory(state, mergePatches(inflight, pending));
}

function isEmptyPatch(patch: ModelsDevSyncMemoryPatch): boolean {
  return (
    !(patch.providerSelections?.length ?? 0) &&
    !(patch.modelSelections?.length ?? 0) &&
    !(patch.quoteProviderChoices?.length ?? 0) &&
    !(patch.viewedModelIds?.length ?? 0)
  );
}

export function useModelsDevSyncMemory() {
  const [memory, setMemory] = useState<ModelsDevSyncMemoryState>(EMPTY_MEMORY);
  const [error, setError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);
  const memoryRef = useRef(memory);
  const pendingPatchRef = useRef<ModelsDevSyncMemoryPatch>(emptyPatch());
  const inflightPatchRef = useRef<ModelsDevSyncMemoryPatch>(emptyPatch());
  const flushPromiseRef = useRef<Promise<void> | null>(null);
  const saveFailedRef = useRef(false);

  const publishMemory = useCallback((next: ModelsDevSyncMemoryState) => {
    memoryRef.current = next;
    setMemory(next);
  }, []);

  const flushMemoryQueue = useCallback(
    (retry = false) => {
      if (retry) saveFailedRef.current = false;
      if (saveFailedRef.current) return Promise.resolve();
      if (flushPromiseRef.current) return flushPromiseRef.current;

      const run = async () => {
        while (!isEmptyPatch(pendingPatchRef.current)) {
          const batch = pendingPatchRef.current;
          pendingPatchRef.current = emptyPatch();
          inflightPatchRef.current = batch;
          setIsSaving(true);
          try {
            const serverState = await updateModelsDevSyncMemory(batch);
            inflightPatchRef.current = emptyPatch();
            publishMemory(overlayUnsavedMemory(serverState, emptyPatch(), pendingPatchRef.current));
            setError(null);
          } catch (saveError) {
            pendingPatchRef.current = mergePatches(batch, pendingPatchRef.current);
            inflightPatchRef.current = emptyPatch();
            saveFailedRef.current = true;
            setError(saveError instanceof Error ? saveError.message : String(saveError));
            break;
          }
        }
        setIsSaving(false);
      };

      const running = run();
      flushPromiseRef.current = running;
      void running.finally(() => {
        if (flushPromiseRef.current === running) flushPromiseRef.current = null;
        if (!saveFailedRef.current && !isEmptyPatch(pendingPatchRef.current)) {
          void flushMemoryQueue();
        }
      });
      return running;
    },
    [publishMemory],
  );

  const queueMemoryPatch = useCallback(
    (patch: ModelsDevSyncMemoryPatch) => {
      publishMemory(patchMemory(memoryRef.current, patch));
      pendingPatchRef.current = mergePatches(pendingPatchRef.current, patch);
      void flushMemoryQueue();
    },
    [flushMemoryQueue, publishMemory],
  );

  const restoreFromServer = useCallback(
    (state: ModelsDevSyncMemoryState) => {
      publishMemory(overlayUnsavedMemory(state, inflightPatchRef.current, pendingPatchRef.current));
    },
    [publishMemory],
  );

  const retry = useCallback(() => flushMemoryQueue(true), [flushMemoryQueue]);

  return { memory, error, isSaving, queueMemoryPatch, restoreFromServer, retry };
}
