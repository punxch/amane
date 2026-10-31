<script setup lang="ts">
import Tooltip from "@/components/common/Tooltip.vue";
import IconButton from "@/components/ui/IconButton.vue";

withDefaults(
  defineProps<{
    label: string;
    variant?: "filled" | "default" | "subtle" | "light";
    size?: "sm" | "md";
    disabled?: boolean;
  }>(),
  { variant: "subtle", size: "md", disabled: false },
);
</script>

<template>
  <Tooltip :label="label">
    <IconButton :label="label" :variant="variant" :size="size" :disabled="disabled">
      <slot />
    </IconButton>
  </Tooltip>
</template>
