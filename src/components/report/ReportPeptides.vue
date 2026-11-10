<script setup lang="ts">
import { computed, onMounted, ref, watch } from "vue";
import PeptideSeq from "../PeptideSeq.vue";
import PeptidoformProfile from "./PeptidoformProfile.vue";
import ReportTable from "../ReportTable.vue";
import SpectrumViewer from "../SpectrumViewer.vue";
import type { PsmKey } from "../../lib/queries/spectrum";
import type { Column } from "../../lib/report";
import type { DatasetInfo } from "../../lib/api";
import { evidence, hasSpectra, proteinExpr, viewScale, type DatasetTables, type ViewScale } from "../../lib/report";
import { PEPTIDOFORM_LIMIT as LIMIT, peptidoformEvidence, searchPeptidoforms } from "../../lib/queries/peptides";
import { modificationCounts } from "../../lib/queries/ptms";
import { fmtBytes, listText } from "../../lib/format";
import { openProtein, state } from "../../store";
import { COL } from "../../lib/qpx";
import { isCancelled } from "../../lib/errors";

const props = defineProps<{ ds: DatasetInfo; t: DatasetTables; fileBase: string; accession?: string }>();

const ev = evidence(props.t);
const prot = ev ? proteinExpr(ev) : null;
const evName = props.t.feature ? "features" : "PSMs";

const q = ref(state.search.peptides);
const mod = ref(state.search.mod);
const mode = ref<"contains" | "exact" | "prefix">("contains");
// Big streamed files: no whole-file scan until the user searches.
const scale = ref<ViewScale | null>(null);
const large = computed(() => !!scale.value?.large);
// Row-group statistics only help when the peptide is the leading sort key
// (relayouted psm views; feature views lead with anchor_protein).
const sorted = computed(() => {
  const first = scale.value?.sortedBy[0];
  return first === COL.peptidoform || first === COL.sequence;
});
const waiting = ref(false);
const modsLoaded = ref(false);
const modsBusy = ref(false);
const modsError = ref("");
const minCharge = ref<number | null>(null);
const busy = ref(false);
const error = ref("");
const rows = ref<Record<string, any>[]>([]);
const truncated = ref(false);
const elapsed = ref(0);
const modNames = ref<string[]>([]);
const table = ref<InstanceType<typeof ReportTable> | null>(null);

// Runs for the peptide selected in the table.
const focus = ref<string | null>(null);
const evRows = ref<Record<string, any>[]>([]);
const evBusy = ref(false);
const pane = ref<"profile" | "list">("profile");

// PSM spectra: when the psm view stores peaks, the evidence pane lists PSMs
// (even if a feature view exists) and a row opens the spectrum viewer.
const spectra = ref(false);
const spectrum = ref<PsmKey | null>(null);
const evSrc = computed(() => (spectra.value && props.t.psm ? props.t.psm : ev));
const evLabel = computed(() => (evSrc.value === props.t.psm ? "PSMs" : "features"));
const evDetail = computed(() => `per-run ${evLabel.value}`);

const cols = computed<Column[]>(() =>
  [
    { key: "peptidoform", label: "Peptidoform" },
    { key: "protein", label: "Protein" },
    { key: "genes", label: "Genes" },
    { key: "charges", label: "Charges", fmt: (v: any) => listText(v, 6) },
    { key: "n_ev", label: evName === "features" ? "Features" : "PSMs", num: true },
    { key: "runs", label: "Runs", num: true },
    { key: "best_pep", label: "Best PEP", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toExponential(1)) },
    { key: "log_int", label: "Mean log10 int.", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toFixed(2)) },
  ].filter((c) => rows.value.length === 0 || c.key in rows.value[0])
);

const evCols = computed<Column[]>(() =>
  [
    { key: "run_file_name", label: "Run" },
    { key: "scan_no", label: "Scan", num: true, fmt: (v: any) => (v == null ? "" : String(v)) },
    { key: "charge", label: "z", num: true },
    { key: "rt_min", label: "RT (min)", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toFixed(2)) },
    { key: "observed_mz", label: "m/z", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toFixed(4)) },
    { key: "mass_error_ppm", label: "ppm", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toFixed(2)) },
    { key: "posterior_error_probability", label: "PEP", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toExponential(1)) },
    { key: "log_int", label: "log10 int.", num: true, fmt: (v: any) => (v == null ? "" : Number(v).toFixed(2)) },
  ].filter((c) => evRows.value.length === 0 || c.key in evRows.value[0])
);

async function search() {
  if (!ev) return;
  waiting.value = false;
  busy.value = true;
  error.value = "";
  focus.value = null;
  const t0 = performance.now();
  try {
    const r = await searchPeptidoforms(ev, prot, { q: q.value, mode: mode.value, mod: mod.value, minCharge: minCharge.value });
    truncated.value = r.truncated;
    rows.value = r.rows;
    elapsed.value = performance.now() - t0;
  } catch (e: any) {
    error.value = String(e?.message ?? e);
  } finally {
    busy.value = false;
  }
}

let evSeq = 0;
async function openPeptide(row: Record<string, any>) {
  if (!ev) return;
  const id = ++evSeq;
  focus.value = row.peptidoform;
  evBusy.value = true;
  evRows.value = [];
  try {
    const r = await peptidoformEvidence(evSrc.value!, String(row.peptidoform));
    if (id !== evSeq) return;
    evRows.value = r.map((x, i) => ({ ...x, _k: i }));
  } finally {
    if (id === evSeq) evBusy.value = false;
  }
}

function openSpectrum(row: Record<string, any>) {
  if (!spectra.value || !focus.value) return;
  spectrum.value = {
    peptidoform: focus.value,
    run_file_name: row.run_file_name ?? null,
    scan: row.scan_no ?? null,
    charge: row.charge ?? null,
    row: row.file_row_number == null ? null : Number(row.file_row_number),
  };
}

watch(
  () => state.search.nonce,
  () => {
    q.value = state.search.peptides;
    mod.value = state.search.mod;
    search();
  }
);

async function loadMods() {
  if (!ev || modsLoaded.value || modsBusy.value || !ev.cols.has(COL.modifications)) return;
  modsBusy.value = true;
  modsError.value = "";
  try {
    modNames.value = (await modificationCounts(props.ds, ev)).map((m) => String(m.name)).filter((x) => x && x !== "null");
    modsLoaded.value = true;
  } catch (e: any) {
    if (!isCancelled(e)) modsError.value = String(e?.message ?? e);
  } finally {
    modsBusy.value = false;
  }
}

onMounted(async () => {
  if (!ev) return;
  hasSpectra(props.t.psm)
    .then((v) => (spectra.value = v && !!props.t.psm?.cols.has(COL.peptidoform)))
    .catch((e) => (error.value = String(e?.message ?? e)));
  try {
    scale.value = await viewScale(ev);
  } catch (e: any) {
    // Without the footer the file's size is unknown: don't scan it blindly.
    error.value = String(e?.message ?? e);
    return;
  }
  if (large.value) {
    // Listing the top peptidoforms or the modifications reads whole columns of
    // the file; on a big streamed file wait for a search term instead. Exact
    // matches skip row groups only when the file is sorted by peptidoform, so
    // don't force the mode otherwise.
    if (sorted.value) mode.value = "exact";
    // The profile scans the whole peptidoform + intensities columns per click;
    // on a large unsorted file open the list pane instead so a click doesn't
    // read the whole file.
    if (!sorted.value) pane.value = "list";
    if (!q.value.trim() && !mod.value) {
      waiting.value = true;
      return;
    }
  } else {
    await loadMods();
  }
  search();
});
</script>

<template>
  <div v-if="!ev" class="muted pad">No feature or PSM view in this dataset.</div>
  <div v-else>
    <form class="bar" @submit.prevent="search">
      <input v-model="q" class="search mono" placeholder="Peptide sequence (e.g. LVNELTEFAK) or peptidoform text (M[Oxidation], UNIMOD:21)…" />
      <select v-model="mode" title="Sequence match">
        <option value="contains">contains</option>
        <option value="prefix">starts with</option>
        <option value="exact">exact</option>
      </select>
      <select
        v-if="modNames.length || (large && ev.cols.has(COL.modifications))"
        v-model="mod"
        :title="modsLoaded ? 'Only peptidoforms carrying this modification' : 'Listing the modifications reads the modifications column of the whole file'"
        @focus="loadMods"
        @mousedown="loadMods"
      >
        <option value="">{{ modsBusy ? "loading modifications…" : "any modification" }}</option>
        <option v-for="m in modNames" :key="m" :value="m">{{ m }}</option>
      </select>
      <span v-if="modsError" class="error">Modifications: {{ modsError }}</span>
      <input v-model.number="minCharge" type="number" min="1" max="9" placeholder="z ≥" class="z" title="Minimum charge" />
      <button class="primary" type="submit" :disabled="busy">Search</button>
      <button class="xs" type="button" :disabled="!rows.length" @click="table?.exportCsv()">CSV</button>
    </form>
    <div class="muted info">
      <span v-if="busy">Searching… <span class="spinner"></span></span>
      <span v-else-if="waiting">
        Large streamed {{ evName }} view ({{ fmtBytes(scale!.bytes) }}, {{ scale!.rows.toLocaleString() }} rows): enter a peptide sequence to search.
        Only the columns and row groups a search needs are fetched{{ sorted ? "; the file is sorted by peptidoform, so exact matches skip most of it" : "; this file is not sorted by peptidoform, so each search reads the whole sequence column" }}.
        Press Search with an empty box to list the top {{ LIMIT }} peptidoforms (reads the whole file's peptide columns).
      </span>
      <span v-else>
        {{ rows.length.toLocaleString() }}{{ truncated ? "+" : "" }} peptidoforms
        <template v-if="truncated"> (top {{ LIMIT }} by {{ evName }} — refine the search)</template>
        · {{ (elapsed / 1000).toFixed(2) }} s · target {{ evName }} · click a peptidoform for its intensity and RT profile across samples and runs, and its {{ spectra ? "PSMs and spectra" : evDetail }}, a protein to open it
      </span>
    </div>
    <div v-if="error" class="error">{{ error }}</div>
    <div v-else class="split" :class="{ open: !!focus, wide: !!focus && pane === 'profile' }">
      <ReportTable
        ref="table"
        :columns="cols"
        :rows="rows"
        row-key="peptidoform"
        :selected="focus"
        clickable
        max-height="calc(100vh - 290px)"
        :file-name="`${fileBase}_peptidoforms${q ? '_' + q : ''}${mod ? '_' + mod : ''}`"
        @row="openPeptide"
      >
        <template #cell-peptidoform="{ row }">
          <PeptideSeq :peptidoform="row.peptidoform" :sequence="row.sequence" :modifications="row.modifications" />
        </template>
        <template #cell-protein="{ value }">
          <a v-if="value" class="mono link" href="#" @click.prevent.stop="openProtein(String(value))">{{ value }}</a>
        </template>
        <template #cell-genes="{ value }"><b>{{ listText(value) }}</b></template>
      </ReportTable>
      <div v-if="focus" class="detail">
        <div class="dhead">
          <PeptideSeq :peptidoform="focus" :sequence="rows.find((r) => r.peptidoform === focus)?.sequence" :modifications="rows.find((r) => r.peptidoform === focus)?.modifications" />
          <button class="xs" @click="focus = null">✕</button>
        </div>
        <div class="tabs">
          <button class="xs" :class="{ on: pane === 'profile' }" @click="pane = 'profile'">Profile</button>
          <button class="xs" :class="{ on: pane === 'list' }" @click="pane = 'list'">{{ evDetail }}</button>
        </div>
        <PeptidoformProfile v-if="pane === 'profile'" :key="focus" :ds="ds" :t="t" :peptidoform="focus" :file-base="fileBase" />
        <div v-if="pane === 'list' && evBusy" class="muted"><span class="spinner"></span></div>
        <div v-if="pane === 'list' && !evBusy" class="muted hint">
          {{ evRows.length.toLocaleString() }}{{ evRows.length >= 5000 ? "+" : "" }} {{ evLabel }}<template v-if="spectra"> · click a PSM to view its annotated spectrum</template>
        </div>
        <ReportTable
          v-if="pane === 'list' && !evBusy"
          :columns="evCols"
          :rows="evRows"
          row-key="_k"
          :clickable="spectra"
          max-height="calc(100vh - 360px)"
          :file-name="`${fileBase}_${focus}_${spectra ? 'psms' : evName}`"
          @row="openSpectrum"
        />
      </div>
    </div>
    <SpectrumViewer
      v-if="spectrum && t.psm"
      :psm="t.psm"
      :target="spectrum"
      :accession="accession"
      :file-base="fileBase"
      @close="spectrum = null"
    />
  </div>
</template>

<style scoped>
.bar {
  display: flex;
  gap: 8px;
  margin-bottom: 6px;
  flex-wrap: wrap;
}
.search {
  flex: 1;
  min-width: 260px;
}
.z {
  width: 70px;
}
.info {
  font-size: 12px;
  margin-bottom: 8px;
}
.split {
  display: grid;
  grid-template-columns: 1fr;
  gap: 14px;
}
.split.open {
  grid-template-columns: minmax(0, 3fr) minmax(0, 2fr);
}
.split.wide {
  grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
}
@media (max-width: 1100px) {
  .split.open {
    grid-template-columns: minmax(0, 1fr);
  }
}
/* A segmented control. */
.tabs {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
  align-self: flex-start;
}
.tabs button {
  border: none;
  border-radius: 0;
  padding: 4px 12px;
  font-size: 13px;
  background: var(--bg-panel);
  color: var(--text-dim);
}
.tabs button + button {
  border-left: 1px solid var(--border);
}
.tabs button.on,
.tabs button.on:hover:not(:disabled) {
  background: var(--accent);
  color: var(--on-accent);
}
.detail {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-panel);
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  gap: 8px;
  min-width: 0;
  align-self: start;
}
.dhead {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
  overflow: hidden;
}
.link {
  color: var(--accent);
  text-decoration: none;
}
.pad {
  padding: 18px;
}
.hint {
  font-size: 12px;
}
</style>
