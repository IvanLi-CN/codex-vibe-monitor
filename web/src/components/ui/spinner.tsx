import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '../../lib/utils'

const spinnerVariants = cva('inline-block animate-spin rounded-full border-current border-r-transparent', {
  variants: {
    size: {
      sm: 'h-4 w-4 border-2',
      md: 'h-[1.15rem] w-[1.15rem] border-[2.5px]',
      lg: 'h-7 w-7 border-[3px]',
    },
  },
  defaultVariants: {
    size: 'md',
  },
})

export interface SpinnerProps
  extends React.HTMLAttributes<HTMLSpanElement>,
    VariantProps<typeof spinnerVariants> {}

function Spinner({ className, size, ...props }: SpinnerProps) {
  return <span className={cn(spinnerVariants({ size }), className)} {...props} />
}

export { Spinner }
