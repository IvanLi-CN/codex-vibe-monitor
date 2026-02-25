import * as React from 'react'
import { cva, type VariantProps } from 'class-variance-authority'
import { cn } from '../../lib/utils'

const alertVariants = cva('flex items-start gap-2 rounded-xl border px-4 py-3 text-sm', {
  variants: {
    variant: {
      default: 'border-base-300/75 bg-base-200/55 text-base-content',
      info: 'border-info/45 bg-info/10 text-info',
      success: 'border-success/45 bg-success/10 text-success',
      warning: 'border-warning/45 bg-warning/14 text-warning',
      error: 'border-error/45 bg-error/12 text-error',
    },
  },
  defaultVariants: {
    variant: 'default',
  },
})

export interface AlertProps extends React.HTMLAttributes<HTMLDivElement>, VariantProps<typeof alertVariants> {}

function Alert({ className, variant, ...props }: AlertProps) {
  return <div className={cn(alertVariants({ variant }), className)} role="status" {...props} />
}

export { Alert }
