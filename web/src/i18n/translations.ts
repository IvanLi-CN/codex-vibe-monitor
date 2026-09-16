import en from "./en.json";
import zh from "./zh.json";

export const supportedLocales = ["zh", "en"] as const;

export type Locale = (typeof supportedLocales)[number];
export type TranslationValues = Record<string, string | number>;

const baseTranslations = { en, zh } as const;

type BaseTranslations = typeof baseTranslations;
type TranslationKey = keyof BaseTranslations["en"];

export const translations: Record<Locale, Record<TranslationKey, string>> = baseTranslations;

export type { TranslationKey };

export const FALLBACK_LOCALE: Locale = "en";

export function formatTranslation(template: string, values?: TranslationValues) {
  if (!values) return template;
  return template.replace(/\{\{\s*(\w+)\s*\}\}/g, (_, key: string) => {
    const value = values[key];
    return value === undefined || value === null ? "" : String(value);
  });
}
