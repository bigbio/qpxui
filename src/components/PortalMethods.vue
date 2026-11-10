<script setup lang="ts">
import { computed } from "vue";
import type { PortalDataset } from "../lib/api";
import { methodTypes } from "../lib/portalMethods";

const props = defineProps<{ dataset: PortalDataset; loading?: boolean }>();
const labelSource = computed(() =>
  `Quantification type derived from declared sample labels: ${props.dataset.quantificationLabels?.join(", ") || "not available"}`
);
</script>

<template>
  <span class="methods">
    <span v-if="loading && !dataset.quantificationTypes?.length && !dataset.metadataError" class="muted">Loading method metadata...</span>
    <template v-else>
      <span :title="labelSource">Quantification: {{ methodTypes(dataset.quantificationTypes).join(", ") }}</span>
      <span title="Acquisition method declared in the run metadata"> · Acquisition: {{ methodTypes(dataset.acquisitionTypes).join(", ") }}</span>
      <span v-if="dataset.metadataError" class="error" :title="dataset.metadataError"> · Methods unavailable</span>
    </template>
  </span>
</template>
