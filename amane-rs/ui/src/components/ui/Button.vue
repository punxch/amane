<script setup lang="ts">
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    variant?: "primary" | "ghost";
    disabled?: boolean;
  }>(),
  { variant: "primary", disabled: false },
);
</script>

<template>
  <button
    :disabled="props.disabled"
    :class="
      cn(
        'inline-flex h-9 items-center justify-center gap-2 rounded-md px-4 text-sm font-medium transition-colors',
        'disabled:pointer-events-none disabled:opacity-50',
        props.variant === 'primary'
          ? 'bg-emerald-500 text-neutral-950 hover:bg-emerald-400'
          : 'border border-neutral-800 text-neutral-300 hover:bg-neutral-900',
      )
    "
  >
    <slot />
  </button>
</template>
