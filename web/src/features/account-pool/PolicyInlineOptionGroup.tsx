import type { CSSProperties } from "react";

interface PolicyInlineOption<T extends string | number> {
  value: T;
  label: string;
}

interface PolicyInlineOptionGroupProps<T extends string | number> {
  ariaLabel: string;
  value: T;
  options: Array<PolicyInlineOption<T>>;
  disabled?: boolean;
  onChange: (value: T) => void;
}

export function PolicyInlineOptionGroup<T extends string | number>({
  ariaLabel,
  value,
  options,
  disabled,
  onChange,
}: PolicyInlineOptionGroupProps<T>) {
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  return (
    <fieldset
      className="policy-inline-radio"
      style={
        {
          "--option-count": options.length,
          "--active-index": activeIndex,
        } as CSSProperties
      }
    >
      <legend className="sr-only">{ariaLabel}</legend>
      <span className="policy-inline-radio-indicator" aria-hidden />
      {options.map((option) => (
        <button
          key={String(option.value)}
          type="button"
          aria-pressed={option.value === value}
          disabled={disabled}
          className="policy-inline-radio-item"
          data-active={option.value === value}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </fieldset>
  );
}
