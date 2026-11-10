<script setup lang="ts">
// The quantms portal source: the dataset registry, and the shared view list
// of the dataset (run) picked in it.
import { onBeforeUnmount, onMounted, ref } from "vue";
import { api, portalRef, type PortalDataset } from "../lib/api";
import PortalDatasetHead from "./portal/PortalDatasetHead.vue";
import PortalRegistry from "./portal/PortalRegistry.vue";
import PortalViews from "./PortalViews.vue";
import { enrichPortalMethods } from "../lib/portalMethods";

const rows = ref<PortalDataset[]>([]);
const selected = ref<PortalDataset | null>(null);
const busy = ref(false);
const error = ref("");
const methodsLoading = ref(false);
const methodsError = ref("");
let unmounted = false;
onBeforeUnmount(() => { unmounted = true; });

async function loadMethods() {
  methodsLoading.value = true;
  methodsError.value = "";
  try {
    await enrichPortalMethods(rows.value, api.portalMethodMetadata, () => {}, () => !unmounted);
  } catch (e: unknown) {
    if (!unmounted) methodsError.value = `Method filters unavailable: ${String(e)}`;
  } finally {
    if (!unmounted) methodsLoading.value = false;
  }
}

async function loadRegistry() {
  busy.value = true;
  error.value = "";
  try {
    rows.value = await api.portalRegistry();
    if (unmounted) return;
    selected.value = rows.value[0] ?? null;
    void loadMethods();
  } catch (e: any) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

function selectDataset(ds: PortalDataset) {
  selected.value = ds;
}

onMounted(loadRegistry);
</script>

<template>
  <div class="portal">
    <h2 class="section-title">Quantms portal: dataset registry</h2>
    <p class="muted">
      Browse, search, and open reanalysed datasets published on the
      <a href="https://portal.quantms.org" target="_blank" rel="noopener">quantms portal</a>.
      Sources load over HTTPS (no AWS credentials needed). <b>Stream</b> the views you choose
      to query them in place (only the byte ranges each query needs are fetched), or
      <b>download them to cache</b> once for fully offline use.
    </p>

    <div v-if="error" class="error">{{ error }}</div>
    <div v-if="busy && !rows.length" class="muted">
      Loading dataset registry… <span class="spinner"></span>
    </div>

    <div v-if="rows.length" class="layout">
      <!-- Left: searchable registry -->
      <PortalRegistry :rows="rows" :selected="selected" :methods-loading="methodsLoading" :methods-error="methodsError" @select="selectDataset" />

      <!-- Right: selected dataset's views -->
      <div class="detail">
        <template v-if="selected">
          <PortalDatasetHead :ds="selected" :methods-loading="methodsLoading" />

          <PortalViews
            :key="portalRef(selected)"
            :accession="selected.accession"
            :dataset-ref="portalRef(selected)"
            :organism="selected.organism"
            :psm-count="selected.psmCount"
          />
        </template>
        <div v-else class="muted">Select a dataset from the registry.</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.portal > p {
  margin: 0;
  max-width: 900px;
  font-size: 14px;
}
.layout {
  display: grid;
  grid-template-columns: minmax(0, 5fr) minmax(0, 7fr);
  gap: 18px;
  margin-top: 16px;
}
.detail {
  border: 1px solid var(--border);
  border-radius: 6px;
  padding: 14px 16px;
  min-height: 360px;
  height: max(360px, calc(100vh - 430px));
  overflow-y: auto;
}
</style>
