<script setup lang="ts">
import Icon, { type IconName } from "@/components/ui/Icon.vue";

withDefaults(
  defineProps<{
    title: string;
    description?: string;
    icon?: IconName | null;
  }>(),
  { description: "", icon: null },
);
</script>

<template>
  <div class="amane-empty">
    <span v-if="icon !== null" class="amane-empty__icon">
      <Icon :name="icon" :size="26" :stroke="1.4" />
    </span>
    <p class="amane-empty__title">{{ title }}</p>
    <p v-if="description !== ''" class="amane-empty__description">{{ description }}</p>
    <div v-if="$slots.default" class="amane-empty__action">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.amane-empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 48px 24px;
  text-align: center;
}

.amane-empty__icon {
  display: inline-flex;
  margin-bottom: 4px;
  color: var(--amane-subtle);
}

.amane-empty__title {
  margin: 0;
  color: var(--amane-fg);
  font-size: 14px;
  font-weight: 600;
}

.amane-empty__description {
  margin: 0;
  max-width: 40ch;
  color: var(--amane-dimmed);
  font-size: 13px;
}

.amane-empty__action {
  margin-top: 8px;
}
</style>
