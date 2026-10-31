<script setup lang="ts">
import { computed } from "vue";

import Tooltip from "@/components/common/Tooltip.vue";
import { connection } from "@/stores/connection";

const STYLES = {
  connected: { color: "var(--amane-teal)", label: "已连接" },
  unreachable: { color: "var(--amane-red)", label: "无法连接后端服务" },
  checking: { color: "var(--amane-yellow)", label: "检查中…" },
} as const;

const current = computed(() => STYLES[connection.status]);
</script>

<template>
  <Tooltip :label="current.label">
    <span
      class="connection-dot"
      role="status"
      :aria-label="current.label"
      :style="{ background: current.color }"
    />
  </Tooltip>
</template>

<style scoped>
.connection-dot {
  display: inline-block;
  width: 10px;
  height: 10px;
  margin: 0 12px;
  border-radius: 50%;
}
</style>
