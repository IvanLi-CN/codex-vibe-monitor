import { type CSSProperties, useId } from "react";

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
  const groupName = useId();
  const activeIndex = Math.max(
    0,
    options.findIndex((option) => option.value === value),
  );

  return (
    <div
      className="policy-inline-radio"
      role="radiogroup"
      aria-label={ariaLabel}
      style={
        {
          "--option-count": options.length,
          "--active-index": activeIndex,
        } as CSSProperties
      }
    >
      <span className="policy-inline-radio-indicator" aria-hidden />
      {options.map((option) => (
        <label
          key={String(option.value)}
          className="policy-inline-radio-item"
          data-active={option.value === value}
          data-disabled={disabled ? "true" : undefined}
        >
          <input
            type="radio"
            name={groupName}
            value={String(option.value)}
            checked={option.value === value}
            disabled={disabled}
            className="sr-only"
            onChange={() => onChange(option.value)}
          />
          <span>{option.label}</span>
        </label>
      ))}
    </div>
  );
}
