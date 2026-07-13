import { useEffect, useMemo, useState } from "react";
import { fetchSettings, type SettingsPayload } from "../lib/api";

function dedupeModels(values: string[]) {
  const seen = new Set<string>();
  const normalized: string[] = [];
  for (const value of values) {
    const trimmed = value.trim();
    if (!trimmed || seen.has(trimmed)) continue;
    seen.add(trimmed);
    normalized.push(trimmed);
  }
  return normalized;
}

export function extractAvailableModelOptions(settings: SettingsPayload | null) {
  if (!settings) return [];
  return dedupeModels(settings.proxy.models ?? []);
}

export function useAvailableModelOptions(enabled = true) {
  const [settings, setSettings] = useState<SettingsPayload | null>(null);

  useEffect(() => {
    if (!enabled) return;
    let cancelled = false;
    void fetchSettings()
      .then((response) => {
        if (!cancelled) {
          setSettings(response);
        }
      })
      .catch(() => {
        if (!cancelled) {
          setSettings(null);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [enabled]);

  return useMemo(() => extractAvailableModelOptions(settings), [settings]);
}
