<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, portalRef, type PortalDataset } from "../lib/api";
import { fmtCount } from "../lib/format";
import PortalViews from "./PortalViews.vue";

const datasets = ref<PortalDataset[]>([]);
const busy = ref(false);
const error = ref("");

const selected = ref<PortalDataset | null>(null);

function isOpen(ds: PortalDataset): boolean {
  return !!selected.value && portalRef(selected.value) === portalRef(ds);
}
// Feature the biological collections (e.g. human cancer cell lines) and
// skip the huge MS-Net reanalysis collection, which otherwise dominates a
// by-sample-count ranking. Take the most-analyzed datasets per collection.
const FEATURED: { name: string; take: number }[] = [
  { name: "celllines", take: 5 },
  { name: "immunopeptidomics", take: 1 },
];

async function loadRecommended() {
  busy.value = true;
  error.value = "";
  try {
    const pages = await Promise.all(
      FEATURED.map(({ name, take }) => api.portalSearch(null, { collection: name, sort: "samples", pageSize: take }))
    );
    // A dataset in both collections is listed once.
    const byRef = new Map<string, PortalDataset>();
    for (const d of pages.flatMap((p) => p.datasets)) if (!byRef.has(portalRef(d))) byRef.set(portalRef(d), d);
    datasets.value = [...byRef.values()].slice(0, 6);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

function toggleDataset(ds: PortalDataset) {
  selected.value = isOpen(ds) ? null : ds;
}

onMounted(() => {
  loadRecommended();
});
</script>

<template>
  <section class="recommended">
    <h2 class="section-title">Recommended datasets</h2>
    <p class="muted intro">
      Biological datasets from the quantms portal: human cancer cell lines and immunopeptidomics.
      Expand one to pick its views, then stream them or download them to the cache.
    </p>

    <div v-if="error" class="error">{{ error }}</div>
    <div v-else-if="busy" class="muted">Loading recommended datasets… <span class="spinner"></span></div>
    <div v-else-if="!datasets.length" class="muted">No featured datasets available.</div>

    <div v-if="datasets.length" class="list">
      <article v-for="ds in datasets" :key="portalRef(ds)" class="row" :class="{ open: isOpen(ds) }">
        <header class="row-head" @click="toggleDataset(ds)">
          <span class="chev" aria-hidden="true">{{ isOpen(ds) ? "▾" : "▸" }}</span>
          <span class="acc mono">{{ ds.accession }}</span>
          <span class="what">
            <span class="title">{{ ds.title || ds.accession }}</span>
            <span class="sub muted">
              <span v-if="ds.organism">{{ ds.organism }}</span>
              <span v-if="ds.instrument"> · {{ ds.instrument }}</span>
              <span v-if="ds.samples"> · {{ ds.samples }} samples</span>
              <span v-if="ds.collectionTitle"> · {{ ds.collectionTitle }}</span>
            </span>
          </span>
          <span class="counts">
            <span title="Proteins"><b>{{ fmtCount(ds.proteins) }}</b> proteins</span>
            <span title="Distinct peptides"><b>{{ fmtCount(ds.peptides) }}</b> peptides</span>
            <span title="PSMs"><b>{{ fmtCount(ds.psmCount) }}</b> PSMs</span>
          </span>
        </header>

        <PortalViews
          v-if="isOpen(ds)"
          class="views"
          :accession="ds.accession"
          :dataset-ref="portalRef(ds)"
          :organism="ds.organism"
          :psm-count="ds.psmCount"
          :catalog="ds"
          featured
        />
      </article>
    </div>
  </section>
</template>

<style scoped>
.intro {
  margin: 0 0 14px;
  font-size: 14px;
}
.list {
  border-top: 1px solid var(--border);
}
.row {
  border-bottom: 1px solid var(--rule);
}
.row.open {
  background: var(--bg-alt);
}
.row-head {
  display: flex;
  align-items: baseline;
  gap: 14px;
  padding: 10px 8px;
  cursor: pointer;
}
.row-head:hover {
  background: var(--hover);
}
.chev {
  color: var(--muted);
  width: 10px;
}
.acc {
  color: var(--accent);
  font-size: 13px;
  font-weight: 500;
  min-width: 92px;
}
.what {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.title {
  font-weight: 600;
}
.sub {
  font-size: 13px;
}
.counts {
  display: flex;
  gap: 18px;
  font-size: 13px;
  color: var(--muted);
  white-space: nowrap;
}
.counts b {
  color: var(--text);
  font-weight: 600;
}
.views {
  padding: 4px 8px 14px 54px;
}
.error {
  margin: 8px 0;
  font-size: 13px;
}
</style>
