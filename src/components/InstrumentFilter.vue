<script setup lang="ts">
import { computed, ref } from "vue";
import { findInstrumentNode, ontologyName, type InstrumentNode } from "../lib/ontology";
import InstrumentTreeNode from "./InstrumentTreeNode.vue";

const props = defineProps<{ modelValue: string; tree: InstrumentNode[]; loading?: boolean; error?: string }>();
const emit = defineEmits<{ "update:modelValue": [id: string]; change: [] }>();
const query = ref("");
const panel = ref<HTMLDetailsElement>();
const label = computed(() =>
  findInstrumentNode(props.tree, props.modelValue)?.name
  ?? (props.modelValue.startsWith("raw:") ? ontologyName(props.modelValue.slice(4)) : "Any instrument")
);
const visibleTree = computed(() => {
  const q = query.value.trim().toLowerCase();
  if (!q) return props.tree;
  const prune = (nodes: InstrumentNode[]): InstrumentNode[] => nodes.flatMap((node) => {
    if (node.name.toLowerCase().includes(q) || node.id.toLowerCase().includes(q)) return [node];
    const children = prune(node.children);
    return children.length ? [{ ...node, children }] : [];
  });
  return prune(props.tree);
});

function select(id: string) {
  emit("update:modelValue", id);
  emit("change");
  if (panel.value) panel.value.open = false;
}
</script>

<template>
  <details ref="panel" class="instrument-filter" @keydown.esc="panel && (panel.open = false)">
    <summary :title="label">{{ label }}</summary>
    <div class="menu">
      <button type="button" class="reset" @click="select('')">Any instrument</button>
      <input v-model="query" type="search" aria-label="Search instrument classes and models" placeholder="Find class or model..." />
      <p v-if="loading" class="muted">Loading PSI-MS instrument hierarchy...</p>
      <p v-if="error" class="error" role="alert">{{ error }} Individual portal instruments remain available.</p>
      <ul v-if="visibleTree.length" class="tree">
        <InstrumentTreeNode
          v-for="node in visibleTree"
          :key="node.id"
          :node="node"
          :selected="modelValue"
          :searching="!!query.trim()"
          @select="select"
        />
      </ul>
      <p v-else-if="!loading" class="muted">No matching instruments.</p>
    </div>
  </details>
</template>

<style scoped>
.instrument-filter {
  position: relative;
  min-width: 180px;
  max-width: 260px;
}
summary {
  cursor: pointer;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-panel);
  padding: 8px 10px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.menu {
  position: absolute;
  z-index: 20;
  top: calc(100% + 4px);
  left: 0;
  width: min(420px, 80vw);
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-panel);
  padding: 10px;
  box-shadow: 0 6px 20px #0004;
}
input {
  width: 100%;
  margin: 8px 0;
}
.tree {
  max-height: 340px;
  overflow: auto;
  list-style: none;
  margin: 0;
  padding: 0;
}
.reset {
  width: 100%;
  text-align: left;
}
p {
  font-size: 12px;
}
</style>
