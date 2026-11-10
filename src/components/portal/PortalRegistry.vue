<script setup lang="ts">
// The portal picker's searchable, sortable list of registry datasets.
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { api, portalRef, type PortalDataset } from "../../lib/api";
import { fmtCount, fmtBytes } from "../../lib/format";
import { REGISTRY_SORTS as SORTS, filterRegistry, nextSort, type RegistrySort } from "../../lib/registry";
import { buildInstrumentTree, ontologyName, selectedInstruments, type InstrumentTerm } from "../../lib/ontology";
import { matchesMethod, methodTypes } from "../../lib/portalMethods";
import InstrumentFilter from "../InstrumentFilter.vue";
import PortalMethods from "../PortalMethods.vue";

const props = defineProps<{ rows: PortalDataset[]; selected: PortalDataset | null; methodsLoading?: boolean; methodsError?: string }>();
const emit = defineEmits<{ (e: "select", ds: PortalDataset): void }>();

const query = ref("");
const sortKey = ref<RegistrySort>("accession");
const sortDesc = ref(false);
const instrument = ref("");
const quantification = ref("");
const acquisition = ref("");
const instrumentTerms = ref<InstrumentTerm[]>([]);
const ontologyLoading = ref(true);
const ontologyError = ref("");
const instrumentTree = computed(() => buildInstrumentTree(
  instrumentTerms.value, [...new Set(props.rows.flatMap((r) => r.instrument ? [r.instrument] : []))],
));
const quantificationTypes = computed(() => [...new Set(props.rows.flatMap((r) => methodTypes(r.quantificationTypes)))].sort());
const acquisitionTypes = computed(() => [...new Set(props.rows.flatMap((r) => methodTypes(r.acquisitionTypes)))].sort());
let unmounted = false;
onBeforeUnmount(() => { unmounted = true; });
onMounted(() => {
  api.portalInstrumentTerms()
    .then((terms) => { if (!unmounted) instrumentTerms.value = terms; })
    .catch((e: unknown) => { if (!unmounted) ontologyError.value = `Instrument hierarchy unavailable: ${String(e)}`; })
    .finally(() => { if (!unmounted) ontologyLoading.value = false; });
});

function setSort(k: RegistrySort) {
  const s = nextSort({ key: sortKey.value, desc: sortDesc.value }, k);
  sortKey.value = s.key;
  sortDesc.value = s.desc;
}

const filtered = computed(() => {
  const instruments = selectedInstruments(instrumentTree.value, instrument.value);
  return filterRegistry(props.rows, query.value, sortKey.value, sortDesc.value).filter((r) =>
    (instruments === null || (!!r.instrument && instruments.includes(r.instrument)))
    && matchesMethod(r.quantificationTypes, quantification.value)
    && matchesMethod(r.acquisitionTypes, acquisition.value)
  );
});
</script>

<template>
  <div class="registry">
    <input
      v-model="query"
      class="search"
      type="search"
      placeholder="Search datasets (accession, title, organism, instrument)…"
    />
    <div class="filters">
      <InstrumentFilter v-model="instrument" :tree="instrumentTree" :loading="ontologyLoading" :error="ontologyError" />
      <select v-model="quantification" :disabled="methodsLoading || !!methodsError" aria-label="Quantification type" title="Quantification family derived from sample labels">
        <option value="">Any quantification</option>
        <option v-for="type in quantificationTypes" :key="type" :value="type">{{ type }}</option>
      </select>
      <select v-model="acquisition" :disabled="methodsLoading || !!methodsError" aria-label="Acquisition type">
        <option value="">Any acquisition</option>
        <option v-for="type in acquisitionTypes" :key="type" :value="type">{{ type }}</option>
      </select>
      <button @click="instrument = ''; quantification = ''; acquisition = ''; query = ''">Reset filters</button>
    </div>
    <div v-if="methodsLoading" class="muted sub">Loading method filters in the background...</div>
    <div v-if="methodsError" class="error" role="alert">{{ methodsError }}</div>
    <div class="sortbar">
      <span class="muted">Sort</span>
      <button
        v-for="s in SORTS"
        :key="s.key"
        class="chip"
        :class="{ on: sortKey === s.key }"
        @click="setSort(s.key)"
        :title="sortKey === s.key ? 'Click to reverse' : `Sort by ${s.label.toLowerCase()}`"
      >
        {{ s.label }}<span v-if="sortKey === s.key"> {{ sortDesc ? "↓" : "↑" }}</span>
      </button>
    </div>
    <div class="meta muted">
      {{ filtered.length }} of {{ rows.length }} datasets · {{ selected?.accession ?? "none" }} selected
    </div>
    <div class="rows">
      <button
        v-for="r in filtered"
        :key="portalRef(r)"
        class="row"
        :class="{ active: selected && portalRef(selected) === portalRef(r) }"
        @click="emit('select', r)"
      >
        <div class="row-top">
          <span class="mono acc">{{ r.accession }}</span>
          <span class="muted size" v-if="r.totalSize != null">{{ fmtBytes(r.totalSize) }}</span>
        </div>
        <div class="title">{{ r.title || "—" }}</div>
        <div class="muted sub" v-if="r.organism || r.instrument">
          <span v-if="r.organism">{{ r.organism }}</span>
          <template v-if="r.organism && r.instrument"> · </template>
          <span v-if="r.instrument">{{ ontologyName(r.instrument) }}</span>
          <template v-if="r.samples"> · {{ r.samples }} samples</template>
        </div>
        <div class="counts">
          <PortalMethods :dataset="r" :loading="methodsLoading" />
          <span :class="{ hi: sortKey === 'peptides' }" title="Distinct peptides">{{ fmtCount(r.peptides) }} pep</span>
          <span :class="{ hi: sortKey === 'psmCount' }" title="PSMs">{{ fmtCount(r.psmCount) }} PSM</span>
          <span :class="{ hi: sortKey === 'proteins' }" title="Proteins">{{ fmtCount(r.proteins) }} prot</span>
          <span v-if="r.collectionTitle" class="coll" :title="r.collectionTitle">{{ r.collectionTitle }}</span>
        </div>
      </button>
      <div v-if="!filtered.length" class="muted empty">No datasets match “{{ query }}”.</div>
    </div>
  </div>
</template>

<style scoped>
.registry {
  display: flex;
  flex-direction: column;
  min-height: 360px;
  height: max(360px, calc(100vh - 430px));
}
.search {
  width: 100%;
}
.filters {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 8px;
}
.meta {
  margin: 8px 0;
  font-size: 13px;
}
.rows {
  flex: 1;
  overflow-y: auto;
  border: 1px solid var(--border);
  border-radius: 6px;
  display: flex;
  flex-direction: column;
}
.row {
  text-align: left;
  padding: 8px 12px;
  border: none;
  border-bottom: 1px solid var(--rule);
  border-radius: 0;
  background: transparent;
  color: var(--text);
  font-weight: 400;
  display: flex;
  flex-direction: column;
  gap: 2px;
  width: 100%;
}
.row:hover:not(:disabled) {
  background: var(--hover);
  color: var(--text);
}
.row.active {
  background: var(--brand-tint);
  box-shadow: inset 3px 0 0 var(--accent);
}
.row-top {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
  gap: 8px;
}
.acc {
  font-family: var(--mono);
  font-weight: 500;
  color: var(--accent);
}
.size {
  font-size: 12px;
  white-space: nowrap;
}
.title {
  font-size: 13px;
  color: var(--text);
  line-height: 1.3;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}
.sub {
  font-size: 12px;
}
.sortbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 4px;
  margin-top: 8px;
  font-size: 12px;
}
.chip {
  padding: 2px 9px;
  font-size: 12px;
  border-radius: 4px;
  font-weight: 400;
}
.chip.on {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--brand-tint);
  font-weight: 600;
}
.counts {
  display: flex;
  flex-wrap: wrap;
  gap: 10px;
  font-size: 12px;
  color: var(--muted);
  font-variant-numeric: tabular-nums;
}
.counts .hi {
  color: var(--text);
  font-weight: 600;
}
.counts .coll {
  margin-left: auto;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  max-width: 45%;
}
.empty {
  padding: 12px;
  font-size: 13px;
}
</style>
