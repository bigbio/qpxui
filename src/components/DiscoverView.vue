<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { api, portalRef, type PortalDataset, type PortalFacets, type PortalSearchResult } from "../lib/api";
import { fmtCount, fmtBytes } from "../lib/format";
import PortalViews from "./PortalViews.vue";
import PeptideSearchPane from "./PeptideSearchPane.vue";

type Tab = "datasets" | "peptides";
const TABS: { key: Tab; label: string }[] = [
  { key: "datasets", label: "Dataset search" },
  { key: "peptides", label: "Peptide & protein search" },
];

const tab = ref<Tab>("datasets");

const PAGE_SIZE = 50;

const facets = ref<PortalFacets>({ organisms: [], instruments: [], collections: [] });
const facetsError = ref("");
const results = ref<PortalSearchResult>({ datasets: [], total: 0, page: 1, totalPages: 1, sort: "" });
const q = ref("");
const organism = ref("");
const instrument = ref("");
const collection = ref("");
const sort = ref("peptides");
const page = ref(1);
const busy = ref(false);
const error = ref("");

const selected = ref<PortalDataset | null>(null);

const SORTS: { key: string; label: string }[] = [
  { key: "peptides", label: "Most peptides" },
  { key: "proteins", label: "Most proteins" },
  { key: "samples", label: "Most samples" },
  { key: "accession", label: "Accession A–Z" },
];

function isOpen(ds: PortalDataset): boolean {
  return !!selected.value && portalRef(selected.value) === portalRef(ds);
}

// Searches overlap (typing, filters, paging); only the latest one may show.
let searchSeq = 0;
async function load() {
  const id = ++searchSeq;
  busy.value = true;
  error.value = "";
  try {
    const res = await api.portalSearch(q.value.trim() || null, {
      collection: collection.value || null,
      organism: organism.value || null,
      instrument: instrument.value || null,
      sort: sort.value || null,
      page: page.value,
      pageSize: PAGE_SIZE,
    });
    if (id !== searchSeq) return;
    results.value = res;
    // Close the expanded dataset (and so cancel its download) if it is no
    // longer on this page.
    if (selected.value) {
      const stillThere = results.value.datasets.some(
        (d) => portalRef(d) === portalRef(selected.value!)
      );
      if (!stillThere) selected.value = null;
    }
  } catch (e: any) {
    if (id === searchSeq) error.value = String(e);
  } finally {
    if (id === searchSeq) busy.value = false;
  }
}

function changeFilter() {
  page.value = 1;
  load();
}

let searchTimer: number | undefined;
function onQueryInput() {
  page.value = 1;
  window.clearTimeout(searchTimer);
  searchTimer = window.setTimeout(load, 350);
}

function gotoPage(p: number) {
  if (p < 1 || p > results.value.totalPages || p === page.value) return;
  page.value = p;
  load();
}

function clearFilters() {
  q.value = "";
  organism.value = "";
  instrument.value = "";
  collection.value = "";
  sort.value = "peptides";
  page.value = 1;
  load();
}

function selectDataset(ds: PortalDataset) {
  selected.value = isOpen(ds) ? null : ds;
}

function clip(list: string[] | null | undefined, n: number): string[] {
  return (list ?? []).slice(0, n);
}
function doiUrl(doi: string): string {
  return /^https?:\/\//i.test(doi) ? doi : `https://doi.org/${doi}`;
}

onMounted(() => {
  api.portalFacets().then((f) => (facets.value = f)).catch((e) => (facetsError.value = String(e?.message ?? e)));
  load();
});

const hasPrev = computed(() => page.value > 1);
const hasNext = computed(() => page.value < results.value.totalPages);
</script>

<template>
  <div class="discover">
    <div class="head">
      <h2 class="section-title">Discover datasets on the quantms portal</h2>
      <p class="muted">
        Find reanalysed QPX datasets published on the
        <a href="https://portal.quantms.org" target="_blank" rel="noopener">quantms portal</a> — by
        metadata, or by the actual peptides / proteins / genes they contain — then pick an interesting
        one to list its QPX views and <b>Stream</b> them lazily or <b>download to cache</b> for offline
        inspection.
      </p>
    </div>

    <div class="tabbar">
      <button
        v-for="t in TABS"
        :key="t.key"
        class="tab"
        :class="{ on: tab === t.key }"
        @click="tab = t.key"
      >
        {{ t.label }}
      </button>
    </div>

    <template v-if="tab === 'datasets'">
    <div class="filters">
      <input
        v-model="q"
        class="search"
        type="search"
        placeholder="Search title, description, keywords, accession…"
        @input="onQueryInput"
      />
      <select v-model="organism" class="sel" @change="changeFilter">
        <option value="">Any organism</option>
        <option v-for="o in facets.organisms" :key="o" :value="o">{{ o }}</option>
      </select>
      <select v-model="instrument" class="sel" @change="changeFilter">
        <option value="">Any instrument</option>
        <option v-for="i in facets.instruments" :key="i" :value="i">{{ i }}</option>
      </select>
      <select v-model="collection" class="sel" @change="changeFilter">
        <option value="">Any collection</option>
        <option v-for="c in facets.collections" :key="c.name" :value="c.name">
          {{ c.title || c.name }}
        </option>
      </select>
      <select v-model="sort" class="sel" @change="changeFilter">
        <option v-for="s in SORTS" :key="s.key" :value="s.key">{{ s.label }}</option>
      </select>
      <button @click="clearFilters">Reset</button>
    </div>

    <div v-if="facetsError" class="error">Filters: {{ facetsError }}</div>
    <div v-if="error" class="error">{{ error }}</div>
    <div v-if="busy && !results.datasets.length" class="muted">
      Searching… <span class="spinner"></span>
    </div>

    <div v-if="results.datasets.length" class="meta muted">
      {{ results.total.toLocaleString() }} datasets · page {{ results.page }} of
      {{ results.totalPages }} · sorted by {{ results.sort || "peptides" }}
    </div>

    <div v-if="results.datasets.length" class="cards">
      <article v-for="ds in results.datasets" :key="portalRef(ds)" class="card" :class="{ open: isOpen(ds) }">
        <header class="card-head" @click="selectDataset(ds)">
          <div class="acc-row">
            <span class="acc">{{ ds.accession }}</span>
            <span v-if="ds.collectionTitle" class="chip" :title="ds.collectionTitle">{{ ds.collectionTitle }}</span>
            <span class="spacer"></span>
            <span v-if="ds.totalSize != null" class="size muted">{{ fmtBytes(ds.totalSize) }}</span>
            <span class="chev">{{ isOpen(ds) ? "▾" : "▸" }}</span>
          </div>
          <h4>{{ ds.title || ds.accession }}</h4>
          <div class="sub muted" v-if="ds.organism || ds.instrument || ds.samples">
            <span v-if="ds.organism">{{ ds.organism }}</span>
            <span v-if="ds.instrument"> · {{ ds.instrument }}</span>
            <span v-if="ds.samples"> · {{ ds.samples }} samples</span>
          </div>
          <div class="counts">
            <span class="stat" title="Proteins"><b>{{ fmtCount(ds.proteins) }}</b> prot</span>
            <span class="stat" title="Distinct peptides"><b>{{ fmtCount(ds.peptides) }}</b> pep</span>
            <span class="stat" title="PSMs"><b>{{ fmtCount(ds.psmCount) }}</b> PSM</span>
          </div>
        </header>

        <div class="meta-body">
          <p v-if="ds.description" class="desc">{{ ds.description }}</p>
          <div v-if="ds.keywords?.length" class="tags">
            <span v-for="k in clip(ds.keywords, 6)" :key="k" class="tag">{{ k }}</span>
            <span v-if="(ds.keywords?.length ?? 0) > 6" class="tag more">+{{ ds.keywords!.length - 6 }}</span>
          </div>
          <div v-if="ds.tissues?.length" class="tags tissue">
            <span v-for="t in clip(ds.tissues, 5)" :key="t" class="tag">{{ t }}</span>
            <span v-if="(ds.tissues?.length ?? 0) > 5" class="tag more">+{{ ds.tissues!.length - 5 }} tissues</span>
          </div>
          <div class="links">
            <a v-if="ds.pxUrl" :href="ds.pxUrl" target="_blank" rel="noopener">PRIDE ↗</a>
            <a v-if="ds.publication?.doi" :href="doiUrl(ds.publication.doi)" target="_blank" rel="noopener">
              DOI ↗
            </a>
            <span v-if="ds.source" class="muted">{{ ds.source }}</span>
          </div>
        </div>

        <PortalViews
          v-if="isOpen(ds)"
          :accession="ds.accession"
          :dataset-ref="portalRef(ds)"
          :organism="ds.organism"
          :psm-count="ds.psmCount"
          :catalog="ds"
          :disabled="busy"
        />
      </article>
    </div>

    <div v-else-if="!busy" class="muted empty">No datasets match your filters.</div>

    <div class="pager" v-if="results.totalPages > 1">
      <button :disabled="!hasPrev" @click="gotoPage(page - 1)">← Prev</button>
      <span class="muted">Page {{ page }} / {{ results.totalPages }}</span>
      <button :disabled="!hasNext" @click="gotoPage(page + 1)">Next →</button>
    </div>
    </template>

    <template v-else-if="tab === 'peptides'">
      <PeptideSearchPane :organisms="facets.organisms" />
    </template>
  </div>
</template>

<style scoped>
.head p {
  margin: 0 0 14px;
  max-width: 900px;
  font-size: 14px;
}
/* Underlined tabs, as on the report page. */
.tabbar {
  display: flex;
  gap: 4px;
  border-bottom: 1px solid var(--border);
}
.tab {
  border: none;
  border-radius: 0;
  background: transparent;
  padding: 8px 14px 9px;
  color: var(--text-dim);
  border-bottom: 3px solid transparent;
  margin-bottom: -1px;
}
.tab:hover:not(:disabled) {
  color: var(--accent);
}
.tab.on {
  color: var(--accent);
  border-bottom-color: var(--accent);
  font-weight: 600;
}
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin: 14px 0 10px;
}
.search {
  flex: 1 1 260px;
  min-width: 200px;
}
.sel {
  max-width: 220px;
}
.meta {
  margin: 6px 0 8px;
  font-size: 13px;
}
.cards {
  border-top: 1px solid var(--border);
  max-height: calc(100vh - 420px);
  overflow-y: auto;
}
.card {
  border-bottom: 1px solid var(--rule);
  padding: 12px 8px;
}
.card.open {
  background: var(--bg-alt);
}
.card-head {
  cursor: pointer;
}
.acc-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.acc {
  font-weight: 500;
  color: var(--accent);
  font-family: var(--mono);
  font-size: 13px;
}
.chip {
  font-size: 12px;
  color: var(--muted);
  max-width: 320px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.chip::before {
  content: "· ";
}
.spacer {
  flex: 1;
}
.size {
  font-size: 12px;
  white-space: nowrap;
}
.chev {
  color: var(--muted);
}
.card h4 {
  margin: 4px 0 2px;
  font-size: 15px;
}
.sub {
  font-size: 13px;
}
.counts {
  display: flex;
  gap: 18px;
  margin-top: 4px;
  font-size: 13px;
  color: var(--muted);
}
.counts b {
  color: var(--text);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.meta-body {
  margin-top: 6px;
}
.desc {
  margin: 0;
  font-size: 13px;
  color: var(--text-dim);
  line-height: 1.5;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.tags {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-top: 6px;
}
.tag {
  font-size: 12px;
  padding: 0 7px;
  border-radius: 3px;
  background: var(--bg-alt);
  border: 1px solid var(--rule);
  color: var(--text-dim);
}
.tag.more {
  color: var(--accent);
}
.tissue .tag {
  font-style: italic;
}
.links {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-top: 6px;
  font-size: 13px;
}
.error {
  margin: 8px 0;
  font-size: 13px;
}
.empty {
  padding: 14px 0;
  font-size: 13px;
}
.pager {
  display: flex;
  align-items: center;
  gap: 12px;
  margin-top: 12px;
}
</style>
