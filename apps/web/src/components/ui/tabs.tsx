// shadcn/ui's Tabs, on Radix primitives.
import { Tabs as T } from 'radix-ui'
import type { ComponentProps } from 'react'
import { cn } from '../../lib/utils.ts'

export const Tabs = T.Root

export function TabsList({ className, ...props }: ComponentProps<typeof T.List>) {
  return (
    <T.List
      className={cn(
        'inline-flex h-8 w-full items-center gap-0.5 rounded-md bg-muted p-0.5',
        className,
      )}
      {...props}
    />
  )
}

export function TabsTrigger({ className, ...props }: ComponentProps<typeof T.Trigger>) {
  return (
    <T.Trigger
      className={cn(
        'h-7 flex-1 rounded-sm text-xs font-medium text-muted-foreground transition-colors outline-none focus-visible:ring-2 focus-visible:ring-ring data-[state=active]:bg-background data-[state=active]:text-foreground data-[state=active]:shadow-sm',
        className,
      )}
      {...props}
    />
  )
}

export function TabsContent({ className, ...props }: ComponentProps<typeof T.Content>) {
  return <T.Content className={cn('mt-3 flex flex-col gap-4 outline-none', className)} {...props} />
}
