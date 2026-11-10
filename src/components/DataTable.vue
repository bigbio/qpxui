<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, shallowRef, watch } from "vue";
import { describeTable, ensureViewReady, type QueryResult } from "../lib/duck";
import { isStructured, toJson } from "../lib/format";
import type { ViewInfo } from "../lib/api";
import type { TableRef } from "../lib/report";
import { viewScale } from "../lib/report";
import { cellFilter, isOptionalColumn, isPeakColumn, tableCount, tablePage } from "../lib/queries/table";
import { columnEnabledDefault, setViewColumnShown, settings, viewColumnShown } from "../lib/settings";
import { state } from "../store";
import ResultCell from "./table/ResultCell.vue";
import CellMenu from "./table/CellMenu.vue";
import HScroll from "./table/HScroll.vue";
import SpectrumViewer from "./SpectrumViewer.vue";
import { psmKeyOf, whyNoSpectrum, type PsmKey } from "../lib/queries/spectrum";
import { isModificationsCol, peptideCells, type PTMSeg } from "../lib/ptm";
import { COL, type ViewKind } from "../lib/qpx";

const props = defineProps<{ view: ViewInfo }>();
const table = ref("");
// Replaced as a whole on each load: no deep proxies over rows and peak arrays.
const result = shallowRef<QueryResult>({ columns: [], rows: [], rowCount: 0, truncated: false });
const total = ref(0);
const page = ref(0);
const pageSize = computed(() => settings.pageSize);
const where = ref("");
const sortCol = ref<string | null>(null);
const sortDir = ref<"ASC" | "DESC">("ASC");
const loading = ref(false);
const error = ref("");
const expanded = ref<Record<string, boolean>>({});
// Toggle rich PTM rendering on/off (default on). When off, peptide/peptidoform
// cells fall back to the plain scalar/structured rendering.
const ptmOn = ref(true);

// Heavy / rarely used columns (peak arrays, cv_params, score lists) are not
// read by default: for streamed files that skips fetching their chunks. Each
// is loadable per view from the "Columns" menu, and the spectrum viewer always
// fetches a single PSM's peaks on demand. `optVis` maps lowercased column name
// -> loaded for this view.
const optVis = ref<Record<string, boolean>>({});
// Large streamed views start with the peak arrays hidden too: a page with them
// reads a whole row group of peaks (often 100+ MB).
const largeRemote = ref(false);
const colTypes = ref<Record<string, string>>({});
const colNames = ref<string[]>([]);
const hasPeakCols = computed(() => colNames.value.some(isPeakColumn));
const hasExtra = computed(() => colNames.value.some((c) => isOptionalColumn(c) && !isPeakColumn(c)));
const extraCols = computed(() => colNames.value.filter((c) => isOptionalColumn(c) && !isPeakColumn(c)));
// The SQL SELECT list: every column except hidden optional ones.
const selectCols = computed(() => colNames.value.filter((c) => !isOptionalColumn(c) || !!optVis.value[c.toLowerCase()]));

let loadSeq = 0;
async function load() {
  const id = ++loadSeq;
  closeMenu();
  loading.value = true;
  error.value = "";
  try {
    table.value = await ensureViewReady(props.view);
    if (!colNames.value.length) {
      const desc = await describeTable(table.value);
      colNames.value = desc.map((d) => d.name);
      colTypes.value = Object.fromEntries(desc.map((d) => [d.name.toLowerCase(), d.type]));
      const vis: Record<string, boolean> = {};
      for (const c of colNames.value) {
        if (!isOptionalColumn(c)) continue;
        // Per-view saved choice wins over the global default.
        const saved = viewColumnShown(props.view.path, c);
        vis[c.toLowerCase()] = saved ?? columnEnabledDefault(c);
      }
      if (hasPeakCols.value && /^(s3|https?):\/\//.test(props.view.path)) {
        largeRemote.value = (await viewScale({ view: props.view, table: table.value })).large;
        if (largeRemote.value) {
          for (const c of colNames.value) if (isPeakColumn(c)) vis[c.toLowerCase()] = false;
        }
      }
      optVis.value = vis;
    }
    const n = await tableCount(table.value, where.value);
    if (id !== loadSeq) return;
    total.value = n;

    const res = await tablePage({
      table: table.value,
      cols: selectCols.value,
      peaks: true,
      where: where.value,
      sort: sortCol.value ? { col: sortCol.value, dir: sortDir.value } : null,
      page: page.value,
      pageSize: pageSize.value,
    });
    if (id === loadSeq) result.value = res;
  } catch (e: any) {
    if (id === loadSeq) error.value = String(e);
  } finally {
    if (id === loadSeq) loading.value = false;
  }
}

onMounted(load);

/** Load or skip an optional column for this view, then reload the page. */
function toggleOptional(col: string) {
  const k = col.toLowerCase();
  optVis.value[k] = !optVis.value[k];
  setViewColumnShown(props.view.path, col, optVis.value[k]);
  load();
}

/** Quick toggle for all peak arrays at once (the spectra), as before. */
const peakCols = computed(() => colNames.value.filter(isPeakColumn));
const peaksVisible = computed(() => peakCols.value.length > 0 && peakCols.value.every((c) => !!optVis.value[c.toLowerCase()]));
function togglePeaks() {
  const on = !peaksVisible.value;
  for (const c of peakCols.value) {
    optVis.value[c.toLowerCase()] = on;
    setViewColumnShown(props.view.path, c, on);
  }
  load();
}

/** Hidden optional columns of this view, in column order: shown as a
 * placeholder column with a "load" affordance instead of silently vanishing. */
const hiddenOptionals = computed(() => colNames.value.filter((c) => isOptionalColumn(c) && !optVis.value[c.toLowerCase()]));

function isPeakCol(j: number): boolean {
  return isPeakColumn(result.value.columns[j] ?? "");
}

// ---- Right-click cell menu ----
const menu = ref<{ x: number; y: number; i: number; j: number } | null>(null);
const msg = ref("");
const spectrum = ref<PsmKey | null>(null);

const psmRef = computed<TableRef | null>(() => {
  if (props.view.view !== ("psm" satisfies ViewKind)) return null;
  const cols = new Set(Object.keys(colTypes.value));
  if (!cols.has(COL.peptidoform) || !cols.has(COL.mz_array) || !cols.has(COL.intensity_array)) return null;
  return { view: props.view, table: table.value, cols, types: colTypes.value, seq: COL.sequence };
});

/** Why the selected row cannot open the spectrum viewer, or "" when it can. */
const spectrumBlock = computed<string>(() => {
  if (!menu.value || !psmRef.value) return "Spectra need a psm view with peptidoform and peak arrays";
  // Peaks read on this page decide the check; when they're hidden the viewer
  // fetches the single PSM's peaks on demand, so opening is still allowed.
  const peaksRead = result.value.columns.some(isPeakColumn);
  return whyNoSpectrum(result.value.columns, result.value.rows[menu.value.i], peaksRead);
});

function openMenu(ev: MouseEvent, i: number, j: number) {
  const w = 220;
  const h = 170;
  menu.value = {
    x: Math.min(ev.clientX, window.innerWidth - w - 8),
    y: Math.min(ev.clientY, window.innerHeight - h - 8),
    i,
    j,
  };
}
function closeMenu() {
  menu.value = null;
}

function viewSpectrum() {
  const m = menu.value;
  const blocked = spectrumBlock.value;
  closeMenu();
  if (!m || blocked) return;
  spectrum.value = psmKeyOf(result.value.columns, result.value.rows[m.i]);
}

function cellText(v: unknown): string {
  return isStructured(v) ? toJson(v) : v == null ? "" : String(v);
}
async function copyCell() {
  const m = menu.value;
  closeMenu();
  if (!m) return;
  await copy(cellText(result.value.rows[m.i]?.[m.j]), "Value copied");
}
async function copyRow() {
  const m = menu.value;
  closeMenu();
  if (!m) return;
  const cols = result.value.columns;
  const row = result.value.rows[m.i] ?? [];
  await copy(toJson(Object.fromEntries(cols.map((c, j) => [c, row[j]])), 2), "Row copied as JSON");
}
async function copy(text: string, ok: string) {
  try {
    await navigator.clipboard.writeText(text);
    flash(ok);
  } catch {
    flash("Copy failed");
  }
}
function flash(s: string) {
  msg.value = s;
  setTimeout(() => {
    if (msg.value === s) msg.value = "";
  }, 1800);
}

function filterBy() {
  const m = menu.value;
  closeMenu();
  if (!m) return;
  const cond = cellFilter(result.value.columns[m.j], result.value.rows[m.i]?.[m.j]);
  where.value = where.value.trim() ? `${where.value.trim()} AND ${cond}` : cond;
}
function expandCell() {
  const m = menu.value;
  closeMenu();
  if (m) toggle(m.i, m.j);
}
function onKey(e: KeyboardEvent) {
  if (e.key === "Escape") closeMenu();
}

// The table's scroll box, with its own horizontal scrollbar.
const hs = ref<InstanceType<typeof HScroll> | null>(null);

/** Scroll a column into view (and flash its header). */
const jumpCol = ref("");
function jumpTo(col: string) {
  jumpCol.value = "";
  const s = hs.value?.el;
  const j = result.value.columns.indexOf(col);
  const th = s?.querySelectorAll("thead th")[j] as HTMLElement | undefined;
  if (!s || !th) return;
  s.scrollLeft = th.offsetLeft - 24;
  th.classList.add("flash-col");
  setTimeout(() => th.classList.remove("flash-col"), 1200);
}

watch(result, () => requestAnimationFrame(() => hs.value?.sync()));

onMounted(() => {
  window.addEventListener("keydown", onKey);
  window.addEventListener("blur", closeMenu);
});
onBeforeUnmount(() => {
  window.removeEventListener("keydown", onKey);
  window.removeEventListener("blur", closeMenu);
});
watch(() => [props.view.path, where.value], () => {
  page.value = 0;
  load();
});
// Re-query when the browse page size changes (from Settings > Advanced).
watch(pageSize, () => {
  page.value = 0;
  load();
});

const totalPages = computed(() => Math.max(0, Math.ceil(total.value / pageSize.value)));
const shown = computed(() => result.value.rows.length);

function prev() {
  if (page.value > 0) {
    page.value--;
    load();
  }
}
function next() {
  if (page.value < totalPages.value - 1) {
    page.value++;
    load();
  }
}
function toggleSort(col: string) {
  if (sortCol.value === col) sortDir.value = sortDir.value === "ASC" ? "DESC" : "ASC";
  else {
    sortCol.value = col;
    sortDir.value = "ASC";
  }
  load();
}
function cellKey(i: number, j: number): string {
  return `${i}:${j}`;
}
function toggle(i: number, j: number) {
  const k = cellKey(i, j);
  expanded.value[k] = !expanded.value[k];
}

// ---- PTM-aware peptide rendering ----
// Per row, a map of cell column index -> peptide tokens (for sequence columns),
// built from the inline `peptidoform` string or from `sequence` + `modifications`.
const peptideMap = computed(() => peptideCells(result.value.columns, result.value.rows));

function segsFor(i: number, j: number): PTMSeg[] | undefined {
  return peptideMap.value[i]?.get(j);
}
function isModsCell(j: number): boolean {
  return isModificationsCol(result.value.columns[j] ?? "");
}
</script>

<template>
  <div class="table-wrap">
    <div class="toolbar">
      <div class="filters">
        <input v-model="where" placeholder="WHERE clause, e.g. charge >= 3" class="mono where" />
        <button @click="page = 0; load()" :disabled="loading">Apply</button>
      </div>
      <label class="ptm-toggle" title="Render peptide PTMs with colour, short codes and mass/accession tooltips">
        <input type="checkbox" v-model="ptmOn" />
        <span>PTMs</span>
      </label>
      <label
        v-if="hasPeakCols"
        class="ptm-toggle"
        :title="largeRemote
          ? 'Large streamed view: peak arrays are hidden because a page with them reads a whole row group of peaks (often 100+ MB). Tick to load them, or right-click a PSM to open its spectrum.'
          : 'Peak arrays (mz_array, intensity_array, …) are heavy and hidden by default. Tick to load them (shown last as \'N peaks\') for this view.'"
      >
        <input type="checkbox" :checked="peaksVisible" @change="togglePeaks" />
        <span>Peak arrays</span>
      </label>
      <details v-if="hasExtra" class="cols-menu">
        <summary :title="largeRemote
          ? 'Extra columns are hidden on large streamed views to avoid reading whole row groups. Load them here for this view.'
          : 'Load rarely used columns for this view. They are skipped by default so the table stays lean.'">
          Columns…
        </summary>
        <div class="cols-pop">
          <label v-for="c in extraCols" :key="c" class="col-row">
            <input type="checkbox" :checked="!!optVis[c.toLowerCase()]" @change="toggleOptional(c)" />
            <span class="mono">{{ c }}</span>
          </label>
          <p class="hint-sm">Defaults come from Settings → “Columns loaded in Browse”. Hidden columns use no bandwidth until loaded.</p>
        </div>
      </details>
      <select
        v-if="result.columns.length > 6"
        v-model="jumpCol"
        class="jump"
        title="Scroll to a column"
        @change="jumpTo(jumpCol)"
      >
        <option value="">Jump to column…</option>
        <option v-for="c in result.columns" :key="c" :value="c">{{ c }}</option>
      </select>
      <div class="muted info">
        <span v-if="msg" class="flash">{{ msg }} · </span>
        {{ total.toLocaleString() }} rows · showing {{ shown.toLocaleString() }}
        <span v-if="result.truncated">(truncated)</span>
      </div>
      <div class="pager">
        <button :disabled="page === 0 || loading" @click="prev">‹ prev</button>
        <span class="muted">{{ page + 1 }} / {{ Math.max(1, totalPages) }}</span>
        <button :disabled="page >= totalPages - 1 || loading" @click="next">next ›</button>
      </div>
    </div>

    <div v-if="error" class="error pad">{{ error }}</div>
    <div v-else-if="loading" class="pad muted">Querying… <span class="spinner"></span></div>
    <HScroll v-else ref="hs">
      <table>
        <thead>
          <tr>
            <th v-for="c in result.columns" :key="c" class="mono" @click="toggleSort(c)">
              {{ c }}
              <span class="sort" v-if="sortCol === c">{{ sortDir === "ASC" ? "▲" : "▼" }}</span>
            </th>
            <th
              v-for="c in hiddenOptionals"
              :key="'h:' + c"
              class="mono opt-hidden"
              :title="`${c} is hidden (not downloaded). Click to load it for this view.`"
              @click="toggleOptional(c)"
            >
              {{ c }} · hidden
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="(row, i) in result.rows" :key="i">
            <td
              v-for="(v, j) in row"
              :key="j"
              :class="{ num: typeof v === 'number' || typeof v === 'bigint', 'mono': typeof v === 'string', active: menu && menu.i === i && menu.j === j }"
              @contextmenu.prevent="openMenu($event, i, j)"
            >
              <ResultCell
                :value="v"
                :expanded="!!expanded[cellKey(i, j)]"
                :segs="segsFor(i, j)"
                :mods="isModsCell(j)"
                :ptm="ptmOn"
                :peak="isPeakCol(j)"
                toggle-titles
                @toggle="toggle(i, j)"
              />
            </td>
            <td
              v-for="c in hiddenOptionals"
              :key="'h:' + c"
              class="opt-hidden"
              :title="`Load ${c} for this view`"
              @click="toggleOptional(c)"
            >
              <span class="load-col">Load {{ c }}</span>
            </td>
          </tr>
        </tbody>
      </table>
    </HScroll>

    <CellMenu
      v-if="menu"
      :x="menu.x"
      :y="menu.y"
      :psm="!!psmRef"
      :spectrum-block="spectrumBlock"
      :value="result.rows[menu.i]?.[menu.j]"
      :column="result.columns[menu.j]"
      :expanded="!!expanded[cellKey(menu.i, menu.j)]"
      @close="closeMenu"
      @spectrum="viewSpectrum"
      @expand="expandCell"
      @copy-cell="copyCell"
      @copy-row="copyRow"
      @filter="filterBy"
    />

    <SpectrumViewer
      v-if="spectrum && psmRef"
      :psm="psmRef"
      :target="spectrum"
      :accession="state.selectedDataset?.prefix"
      :file-base="state.selectedDataset?.prefix"
      @close="spectrum = null"
    />
  </div>
</template>

<style scoped>
/* The table scrolls inside a box bounded by the window, so the horizontal
   scrollbar always sits at the bottom of the panel, not after the last row. */
.table-wrap {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-height: 0;
}
.toolbar {
  display: flex;
  align-items: center;
  gap: 14px;
  padding: 10px 18px;
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}
.filters {
  display: flex;
  gap: 8px;
  flex: 1;
}
.where {
  flex: 1;
  font-size: 12px;
}
.info {
  font-size: 12px;
}
.ptm-toggle {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 13px;
  font-weight: 500;
  color: var(--text-dim);
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
}
.ptm-toggle input {
  accent-color: var(--accent);
  cursor: pointer;
}
.pager {
  display: flex;
  align-items: center;
  gap: 8px;
}
.jump {
  font-size: 12px;
  max-width: 190px;
}
/* "Columns…" menu for loading rarely used columns per view. */
.cols-menu {
  position: relative;
  font-size: 12px;
}
.cols-menu summary {
  cursor: pointer;
  user-select: none;
  color: var(--text-dim);
  list-style: none;
  white-space: nowrap;
}
.cols-menu summary::-webkit-details-marker {
  display: none;
}
.cols-menu summary:hover {
  color: var(--accent);
}
.cols-pop {
  position: absolute;
  top: calc(100% + 6px);
  left: 0;
  z-index: 20;
  min-width: 240px;
  max-height: 320px;
  overflow: auto;
  background: var(--bg-panel);
  border: 1px solid var(--border);
  border-radius: 6px;
  box-shadow: var(--shadow);
  padding: 6px 8px;
}
.col-row {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 4px;
  font-size: 12.5px;
  cursor: pointer;
  white-space: nowrap;
}
.col-row input {
  accent-color: var(--accent);
  cursor: pointer;
}
.hint-sm {
  font-size: 11.5px;
  color: var(--muted);
  margin: 6px 2px 2px;
  line-height: 1.4;
}
th.flash-col {
  background: var(--brand-tint);
  transition: background 0.3s;
}
td.active {
  outline: 1px solid var(--accent);
  outline-offset: -1px;
}
.flash {
  color: var(--accent);
}
table {
  border-collapse: collapse;
  width: max-content;
  min-width: 100%;
}
th,
td {
  text-align: left;
  padding: 6px 12px;
  border-bottom: 1px solid var(--rule);
  white-space: nowrap;
  font-size: 12.5px;
}
th {
  position: sticky;
  top: 0;
  background: var(--bg-alt);
  border-bottom-color: var(--border);
  font-weight: 500;
  cursor: pointer;
  user-select: none;
  z-index: 1;
}
th:hover {
  color: var(--accent);
}
/* Hidden optional columns: a distinguishable placeholder you can click to load. */
th.opt-hidden,
td.opt-hidden {
  color: var(--muted);
  background: repeating-linear-gradient(
    -45deg,
    var(--bg-alt),
    var(--bg-alt) 6px,
    transparent 6px,
    transparent 12px
  );
  cursor: pointer;
}
th.opt-hidden:hover,
td.opt-hidden:hover {
  color: var(--accent);
  background-color: var(--brand-tint);
}
.load-col {
  font-size: 11.5px;
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 1px 8px;
}
td.opt-hidden:hover .load-col {
  border-color: var(--accent);
}
.sort {
  font-size: 10px;
  color: var(--accent);
}
td.num {
  font-variant-numeric: tabular-nums;
}
.pad {
  padding: 18px;
}
</style>
