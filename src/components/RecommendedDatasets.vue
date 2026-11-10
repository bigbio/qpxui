<script setup lang="ts">
import { onMounted, ref } from "vue";
import { api, portalRef, type PortalDataset } from "../lib/api";
import { LARGE_BYTES, pickExamples } from "../lib/examples";
import { fmtBytes, fmtCount } from "../lib/format";
import PortalViews from "./PortalViews.vue";

const groups = ref<ReturnType<typeof pickExamples>>([]);
const busy = ref(false);
const error = ref("");

const selected = ref<PortalDataset | null>(null);

function isOpen(ds: PortalDataset): boolean {
  return !!selected.value && portalRef(selected.value) === portalRef(ds);
}

const large = (ds: PortalDataset) => (ds.totalSize ?? 0) >= LARGE_BYTES;

async function loadExamples() {
  busy.value = true;
  error.value = "";
  try {
    groups.value = pickExamples(await api.portalRegistry());
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
  loadExamples();
});
</script>

<template>
  <section class="recommended">
    <h2 class="section-title">Example datasets</h2>
    <p class="muted intro">
      Reanalysed datasets from the quantms portal. Expand one to pick its views, then stream them or download them to
      the cache.
    </p>

    <div v-if="error" class="error">{{ error }}</div>
    <div v-else-if="busy" class="muted">Loading examples… <span class="spinner"></span></div>
    <div v-else-if="!groups.length" class="muted">No example datasets available.</div>

    <div v-for="g in groups" :key="g.group.title" class="group">
      <h3 class="group-title">{{ g.group.title }} <span class="muted group-intro">{{ g.group.intro }}</span></h3>
      <div class="list">
        <article v-for="{ example, dataset: ds } in g.items" :key="portalRef(ds)" class="row" :class="{ open: isOpen(ds) }">
          <header class="row-head" @click="toggleDataset(ds)">
            <span class="chev" aria-hidden="true">{{ isOpen(ds) ? "▾" : "▸" }}</span>
            <span class="acc mono">{{ ds.accession }}</span>
            <span class="what">
              <span class="title">{{ example.note }}</span>
              <span class="sub muted">
                <span v-if="ds.organism">{{ ds.organism }}</span>
                <span v-if="ds.instrument"> · {{ ds.instrument }}</span>
                <span v-if="ds.samples"> · {{ ds.samples }} samples</span>
              </span>
            </span>
            <span class="counts">
              <span v-if="ds.proteins" title="Proteins"><b>{{ fmtCount(ds.proteins) }}</b> proteins</span>
              <span v-if="ds.peptides" title="Distinct peptides"><b>{{ fmtCount(ds.peptides) }}</b> peptides</span>
              <span v-if="ds.psmCount" title="PSMs"><b>{{ fmtCount(ds.psmCount) }}</b> PSMs</span>
              <span v-if="ds.totalSize" class="size" :class="{ large: large(ds) }" :title="large(ds) ? 'Large: stream it rather than downloading' : 'Total size of the dataset'">
                {{ fmtBytes(ds.totalSize) }}
              </span>
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
    </div>
  </section>
</template>

<style scoped>
.intro {
  margin: 0 0 14px;
  font-size: 14px;
}
.group {
  margin-bottom: 18px;
}
.group-title {
  font-size: 14px;
  font-weight: 600;
  margin: 0 0 6px;
}
.group-intro {
  font-weight: 400;
  font-size: 13px;
  margin-left: 6px;
}
.list {
  border-top: 1px solid var(--border);
}
.size {
  min-width: 64px;
  text-align: right;
}
.size.large {
  color: var(--warn);
  font-weight: 600;
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
