import type { InputHTMLAttributes, SelectHTMLAttributes } from 'react'

type NativeAutocompleteProps = 'autoComplete'
type TextAssistProps = NativeAutocompleteProps | 'autoCapitalize' | 'autoCorrect' | 'spellCheck'

export type NativeFormAutocompleteOffProps = Pick<SelectHTMLAttributes<HTMLSelectElement>, NativeAutocompleteProps>
export type TextInputAutocompleteOffProps = Pick<InputHTMLAttributes<HTMLInputElement>, TextAssistProps>

export const nativeFormAutocompleteOffProps = {
  autoComplete: 'off',
} satisfies NativeFormAutocompleteOffProps

export const textInputAutocompleteOffProps = {
  ...nativeFormAutocompleteOffProps,
  autoCapitalize: 'none',
  autoCorrect: 'off',
  spellCheck: false,
} satisfies TextInputAutocompleteOffProps

export function resolveTextInputAutocompleteProps(
  overrides?: Partial<TextInputAutocompleteOffProps>,
): TextInputAutocompleteOffProps {
  return {
    ...textInputAutocompleteOffProps,
    ...overrides,
  }
}
