import { cva, type VariantProps } from 'class-variance-authority'

export const formControlSizeVariants = cva('', {
  variants: {
    size: {
      sm: 'h-8 rounded-md px-3 text-sm',
      default: 'h-10 rounded-lg px-3 text-sm',
      filter: 'h-10 rounded-lg px-3 text-sm',
    },
  },
  defaultVariants: {
    size: 'default',
  },
})

export type FormControlSize = NonNullable<VariantProps<typeof formControlSizeVariants>['size']>

export const formFieldSpanVariants = cva('', {
  variants: {
    size: {
      compact: 'xl:col-span-2',
      wide: 'xl:col-span-3',
      full: 'xl:col-span-12',
    },
  },
})

export type FormFieldSpanSize = NonNullable<VariantProps<typeof formFieldSpanVariants>['size']>
