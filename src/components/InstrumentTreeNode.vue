<script setup lang="ts">
import type { InstrumentNode } from "../lib/ontology";

defineProps<{ node: InstrumentNode; selected: string; searching: boolean }>();
const emit = defineEmits<{ select: [id: string] }>();
</script>

<template>
  <li>
    <div class="node">
      <details v-if="node.children.length" :open="searching || node.id === 'MS:1000463'">
        <summary>
          <button
            type="button"
            :class="{ selected: selected === node.id }"
            :aria-pressed="selected === node.id"
            :title="node.id"
            @click.prevent="emit('select', node.id)"
          >{{ node.name }}</button>
        </summary>
        <ul>
          <InstrumentTreeNode
            v-for="child in node.children"
            :key="child.id"
            :node="child"
            :selected="selected"
            :searching="searching"
            @select="emit('select', $event)"
          />
        </ul>
      </details>
      <button
        v-else
        type="button"
        :class="{ selected: selected === node.id }"
        :aria-pressed="selected === node.id"
        :title="node.id"
        @click="emit('select', node.id)"
      >{{ node.name }}</button>
    </div>
  </li>
</template>

<style scoped>
ul {
  list-style: none;
  margin: 0;
  padding-left: 18px;
}
summary {
  cursor: pointer;
}
button {
  padding: 4px 7px;
  border-color: transparent;
  background: transparent;
  text-align: left;
  font-size: 12px;
}
button.selected {
  border-color: var(--accent);
  color: var(--accent);
}
.node {
  margin: 2px 0;
}
</style>
