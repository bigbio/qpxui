<script setup lang="ts">
// One configurable folder in Settings (download cache, log): where it is,
// Change… (native folder picker) and Reset to default.
import { computed, ref } from "vue";
import { api, type StoragePaths } from "../lib/api";

const props = defineProps<{
  /** The folder in use and its default. */
  current: string | null;
  fallback: string | null;
  /** Saves the choice (null: the default) and returns the new paths. */
  apply: (path: string | null) => Promise<StoragePaths>;
}>();
const emit = defineEmits<{ changed: [StoragePaths] }>();

const busy = ref(false);
const error = ref("");
const isDefault = computed(() => !!props.current && props.current === props.fallback);

async function set(path: string | null) {
  busy.value = true;
  error.value = "";
  try {
    emit("changed", await props.apply(path));
  } catch (e: any) {
    error.value = String(e?.message ?? e);
  } finally {
    busy.value = false;
  }
}

async function change() {
  const picked = await api.chooseFolder(props.current ?? undefined);
  if (picked) await set(picked);
}
</script>

<template>
  <div class="folder">
    <div class="row">
      <span class="mono path" :title="current ?? ''">{{ current ?? "not available" }}</span>
      <span v-if="isDefault" class="hint-sm">(default)</span>
    </div>
    <div class="row">
      <button class="xs" :disabled="busy" @click="change">Change…</button>
      <button v-if="!isDefault && fallback" class="xs" :disabled="busy" @click="set(null)">Reset to default</button>
      <span v-if="busy" class="spinner"></span>
    </div>
    <div v-if="error" class="error">{{ error }}</div>
  </div>
</template>

<style scoped>
.folder {
  display: flex;
  flex-direction: column;
  gap: 6px;
  margin-bottom: 8px;
}
.row {
  display: flex;
  gap: 8px;
  align-items: baseline;
  flex-wrap: wrap;
}
.path {
  font-size: 12px;
  word-break: break-all;
}
.hint-sm {
  font-size: 12px;
  color: var(--muted);
}
.error {
  font-size: 13px;
}
</style>
