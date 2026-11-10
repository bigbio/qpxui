<script setup lang="ts">
import { ontologyName } from "../lib/ontology";
import { computed, onMounted, ref } from "vue";
import { api, type PsHit, type PsModification, type PsSearchResult } from "../lib/api";
import { fmtCount } from "../lib/format";
import PortalViews from "./PortalViews.vue";

defineProps<{ organisms: string[] }>();

type SearchType = "auto" | "peptide" | "protein" | "gene";

const query = ref("");
const explicitType = ref<SearchType>("auto");
const matchMode = ref("exact");
const organism = ref("");
const mod = ref("");
const mods = ref<PsModification[]>([]);
const modsError = ref("");

const result = ref<PsSearchResult | null>(null);
const busy = ref(false);
/** The open hit is downloading; like a search, it holds the Search button. */
const viewsDownloading = ref(false);
const error = ref("");

const expanded = ref<PsHit | null>(null);

const TYPES: { key: SearchType; label: string }[] = [
  { key: "auto", label: "Auto" },
  { key: "peptide", label: "Peptide" },
  { key: "protein", label: "Protein" },
  { key: "gene", label: "Gene" },
];
const MATCHES = [
  { key: "exact", label: "exact sequence" },
  { key: "contains", label: "contains (substring)" },
];
const showMatch = computed(() => explicitType.value === "auto" || explicitType.value === "peptide");

/** A hit's run ref (the portal leaves it empty for some hits). */
function hitRef(h: PsHit): string {
  return h.datasetRef || h.datasetAccession;
}
function isExpanded(h: PsHit): boolean {
  return !!expanded.value && hitRef(expanded.value) === hitRef(h);
}

// Enter can start a search while another runs; only the latest one may show.
let searchSeq = 0;
async function runSearch() {
  const q = query.value.trim();
  if (!q) return;
  const id = ++searchSeq;
  busy.value = true;
  error.value = "";
  // The new hits replace the open one (closing it cancels its download).
  expanded.value = null;
  try {
    const res = await api.psSearch(q, {
      dtype: explicitType.value === "auto" ? null : explicitType.value,
      matchMode: explicitType.value === "protein" || explicitType.value === "gene" ? null : matchMode.value || null,
      organism: organism.value || null,
      modification: mod.value || null,
      limit: 50,
    });
    if (id === searchSeq) result.value = res;
  } catch (e: any) {
    if (id === searchSeq) error.value = String(e);
  } finally {
    if (id === searchSeq) busy.value = false;
  }
}

function toggleHit(h: PsHit) {
  expanded.value = isExpanded(h) ? null : h;
}

onMounted(async () => {
  try {
    mods.value = await api.psModifications();
  } catch (e: any) {
    modsError.value = String(e?.message ?? e);
  }
});
</script>

<template>
  <div class="ppane">
    <div class="filters">
      <div class="typebar">
        <button
          v-for="t in TYPES"
          :key="t.key"
          class="type"
          :class="{ on: explicitType === t.key }"
          @click="explicitType = t.key"
        >
          {{ t.label }}
        </button>
      </div>
      <input
        v-model="query"
        class="search"
        type="search"
        :placeholder="explicitType === 'peptide' ? 'Peptide sequence, e.g. AVGKVLPELNGK…' : 'Peptide, protein or gene — e.g. GAPDH, P04406, AVGKVLPELNGK…'"
        @keydown.enter="runSearch"
      />
      <button class="primary" @click="runSearch" :disabled="busy || viewsDownloading || !query.trim()">
        {{ busy ? "Searching…" : "Search" }}
      </button>
    </div>

    <div class="subfilters">
      <select v-if="showMatch" v-model="matchMode" class="sel">
        <option v-for="m in MATCHES" :key="m.key" :value="m.key">{{ m.label }}</option>
      </select>
      <select v-model="organism" class="sel">
        <option value="">Any organism</option>
        <option v-for="o in organisms" :key="o" :value="o">{{ o }}</option>
      </select>
      <select v-model="mod" class="sel">
        <option value="">Any modification</option>
        <option v-for="m in mods" :key="m.name" :value="m.name">
          {{ m.name }} ({{ m.nDatasets }} datasets)
        </option>
      </select>
    </div>

    <div v-if="modsError" class="error">Modifications: {{ modsError }}</div>

    <p class="muted hint">
      Finds which quantms datasets contain the searched peptide, protein or gene across
      the whole portal — then expand a hit to list its QPX views and
      <b>Stream</b> (lazy) or <b>download to cache</b>.
    </p>

    <div v-if="error" class="error">{{ error }}</div>

    <div v-if="busy && !result" class="muted">Searching… <span class="spinner"></span></div>

    <template v-if="result">
      <div class="meta muted">
        {{ result.totalDatasets.toLocaleString() }} dataset(s)
        <template v-if="!result.unmapped"> · sorted by number of peptides</template>
      </div>
      <div v-if="result.unmapped" class="note">
        Query didn't map cleanly to a known protein; showing the closest matches.
      </div>

      <div v-if="result.datasets.length" class="hits">
        <article v-for="h in result.datasets" :key="hitRef(h)" class="hit" :class="{ open: isExpanded(h) }">
          <header class="hit-head" @click="toggleHit(h)">
            <div class="acc-row">
              <span class="acc">{{ h.datasetAccession }}</span>
              <span v-if="h.collection" class="chip">{{ h.collection }}</span>
              <span class="spacer"></span>
              <span class="chev">{{ isExpanded(h) ? "▾" : "▸" }}</span>
            </div>
            <div class="sub muted">
              <span v-if="h.organism">{{ h.organism }}</span>
              <span v-if="h.organismPart"> · {{ h.organismPart }}</span>
              <span v-if="h.instrument"> · {{ ontologyName(h.instrument) }}</span>
            </div>
            <div class="counts">
              <span class="stat" title="Distinct peptides"><b>{{ fmtCount(h.nPeptides) }}</b> pep</span>
              <span class="stat" title="Peptidoforms"><b>{{ fmtCount(h.nPeptidoforms) }}</b> peptidoforms</span>
              <span class="stat" title="Total observations"><b>{{ fmtCount(h.totalObs) }}</b> obs</span>
            </div>
          </header>

          <div v-if="h.samplePeptidoforms.length && !isExpanded(h)" class="sample">
            <span v-for="(pf, i) in h.samplePeptidoforms.slice(0, 3)" :key="i" class="mono pf">{{ pf }}</span>
          </div>

          <PortalViews
            v-if="isExpanded(h)"
            :accession="h.datasetAccession"
            :dataset-ref="hitRef(h)"
            :organism="h.organism"
            :disabled="busy"
            @downloading="viewsDownloading = $event"
          />
        </article>
      </div>
      <div v-else class="muted empty">No datasets match this search.</div>
    </template>
  </div>
</template>

<style scoped>
.ppane {
  display: flex;
  flex-direction: column;
}
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  align-items: center;
}
/* A segmented control. */
.typebar {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
}
.type {
  padding: 6px 14px;
  font-size: 13px;
  border: none;
  border-radius: 0;
  background: var(--bg-panel);
  color: var(--text-dim);
}
.type + .type {
  border-left: 1px solid var(--border);
}
.type.on,
.type.on:hover:not(:disabled) {
  background: var(--accent);
  color: var(--on-accent);
}
.search {
  flex: 1 1 280px;
  min-width: 200px;
}
.subfilters {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
}
.sel {
  max-width: 260px;
}
.hint {
  font-size: 13px;
  margin: 8px 0;
}
.meta {
  margin: 6px 0 8px;
  font-size: 13px;
}
.note {
  font-size: 13px;
  color: var(--warn);
  margin-bottom: 8px;
}
.hits {
  border-top: 1px solid var(--border);
  max-height: calc(100vh - 500px);
  overflow-y: auto;
}
.hit {
  border-bottom: 1px solid var(--rule);
  padding: 10px 8px;
}
.hit.open {
  background: var(--bg-alt);
}
.hit-head {
  cursor: pointer;
}
.acc-row {
  display: flex;
  align-items: center;
  gap: 8px;
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
}
.chip::before {
  content: "· ";
}
.spacer {
  flex: 1;
}
.chev {
  color: var(--muted);
}
.sub {
  font-size: 13px;
  margin-top: 2px;
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
.sample {
  display: flex;
  flex-wrap: wrap;
  gap: 4px 12px;
  margin-top: 8px;
}
.pf {
  font-size: 12px;
  color: var(--text-dim);
}
.error {
  margin: 8px 0;
  font-size: 13px;
}
.empty {
  padding: 12px 0;
  font-size: 13px;
}
</style>
