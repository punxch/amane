<script setup lang="ts">
import { computed } from "vue";
import { RouterLink, useRoute } from "vue-router";

import { isNavActive, type NavItem } from "@/components/layout/nav";
import Icon from "@/components/ui/Icon.vue";

const props = defineProps<{ item: NavItem }>();

const route = useRoute();
const active = computed(() => isNavActive(route.path, props.item));
</script>

<template>
  <RouterLink
    :to="props.item.to"
    class="nav-link"
    :data-active="active || undefined"
    :aria-current="active ? 'page' : undefined"
  >
    <Icon :name="props.item.icon" :size="18" />
    <span class="nav-link__label">{{ props.item.label }}</span>
  </RouterLink>
</template>

<style scoped>
.nav-link {
  display: flex;
  align-items: center;
  gap: 10px;
  height: 38px;
  padding: 0 12px;
  border-radius: var(--amane-radius);
  color: var(--amane-dimmed);
  font-size: 14px;
  font-weight: 500;
  white-space: nowrap;
  transition:
    background-color 100ms ease,
    color 100ms ease;
}

.nav-link:hover {
  background: var(--amane-surface-2);
  color: var(--amane-bright);
}

.nav-link[data-active="true"] {
  background: var(--amane-brand);
  color: #fff;
}

.nav-link:focus-visible {
  outline: 2px solid var(--amane-brand-4);
  outline-offset: 1px;
}

.nav-link__label {
  overflow: hidden;
  text-overflow: ellipsis;
}
</style>
