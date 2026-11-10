<script setup lang="ts">
// The QPX views of one portal dataset: list them, tick some, then stream them
// in place or download them to the cache. Unmounting (the dataset closed, or
// hidden by a new search) cancels a running download.
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { api, type CatalogStats, type DatasetInfo, type ViewInfo } from "../lib/api";
import { loadDatasets } from "../store";
import { confirmDownload, totalBytes, useDownload } from "../lib/downloads";
import { fmtBytes } from "../lib/format";
import DownloadProgress from "./DownloadProgress.vue";

const props = defineProps<{
  accession: string;
  /** The run's ref: one accession can hold several runs. */
  datasetRef: string;
  organism?: string | null;
  psmCount?: number | null;
  /** The dataset's counts in the portal catalogue, if known. */
  catalog?: CatalogStats | null;
  /** The recommended strip: with no view ticked, actions take all views. */
  featured?: boolean;
  /** The parent list is searching: hold the actions until it is done. */
  disabled?: boolean;
}>();
const emit = defineEmits<{ downloading: [boolean] }>();

const views = ref<ViewInfo[]>([]);
const checked = ref<Record<string, boolean>>({});
const listing = ref(true);
const downloading = ref(false);
const error = ref("");
const { id: downloadId, cancelling, files: downloadFiles, cancel, run: runDownload, unmounted } = useDownload();

const ticked = computed(() => views.value.filter((v) => checked.value[v.path]));
/** The views Stream and Download act on. */
const chosen = computed(() => (props.featured && !ticked.value.length ? views.value : ticked.value));

watch(downloading, (d) => emit("downloading", d));
onBeforeUnmount(() => {
  if (downloading.value) emit("downloading", false);
});

onMounted(async () => {
  try {
    views.value = await api.portalListViews(props.datasetRef, props.organism, props.psmCount);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    listing.value = false;
  }
});

function toggleAll() {
  const allOn = views.value.every((v) => checked.value[v.path]);
  for (const v of views.value) checked.value[v.path] = !allOn;
}

/** Views whose rows the catalogue counts describe. */
const EVIDENCE = new Set(["psm", "feature"]);

function open(list: ViewInfo[], label: string) {
  const names = new Set(list.map((v) => v.name));
  // The catalogue counts the whole dataset: keep them only when every
  // evidence file is open, so they match what the report shows.
  const whole = views.value.filter((v) => EVIDENCE.has(v.view)).every((v) => names.has(v.name));
  const ds: DatasetInfo = {
    prefix: props.accession,
    path: `quantms portal / ${props.datasetRef}`,
    views: list.sort((a, b) => a.view.localeCompare(b.view)),
    ...(whole && props.catalog ? { catalog: props.catalog } : {}),
  };
  loadDatasets([ds], label);
}

function stream() {
  error.value = "";
  open(chosen.value.map((v) => ({ ...v })), `portal | ${props.accession} (stream)`);
}

async function download() {
  const list = chosen.value;
  downloading.value = true;
  error.value = "";
  try {
    if (!(await confirmDownload(list, "portal"))) return;
    // Closed while the dialog was open: don't start a transfer for it.
    if (unmounted()) return;
    const local = await runDownload((id) => api.portalCacheViews(list.map((v) => v.path), id));
    // Closed meanwhile (which also cancels): say nothing.
    if (unmounted()) return;
    if (!local) {
      error.value = "Download cancelled; partial files were removed.";
      return;
    }
    open(local, `portal | ${props.accession}`);
  } catch (e: any) {
    error.value = String(e);
  } finally {
    downloading.value = false;
  }
}
</script>

<template>
  <div class="loadbox">
    <div class="toolbar">
      <span class="muted">
        {{ views.length }} view(s) · {{ ticked.length }} selected<template v-if="ticked.length"> · {{ fmtBytes(totalBytes(ticked)) }}</template>
      </span>
      <div class="actions">
        <button
          @click="stream"
          :disabled="disabled || downloading || !chosen.length"
          title="Query the files in place — each query fetches only the byte ranges it needs"
        >
          Stream
        </button>
        <button class="primary" @click="download" :disabled="disabled || downloading || !chosen.length">
          {{ downloading ? "Downloading…" : "Download to cache" }}
        </button>
      </div>
    </div>

    <div v-if="error" class="error">{{ error }}</div>

    <DownloadProgress
      v-if="downloadId"
      :files="downloadFiles"
      :cancelling="cancelling"
      label="Transferring to cache…"
      compact
      @cancel="cancel"
    />

    <label class="all">
      <input type="checkbox" :checked="views.length > 0 && ticked.length === views.length" @change="toggleAll" />
      <span>All views</span>
    </label>
    <div v-if="views.length" class="view-list">
      <label v-for="v in views" :key="v.path" class="view">
        <input v-model="checked[v.path]" type="checkbox" />
        <span class="mono view-name">{{ v.name }}</span>
        <span class="muted">{{ fmtBytes(v.sizeBytes) }}</span>
      </label>
    </div>
    <div v-else-if="listing" class="muted">Loading views… <span class="spinner"></span></div>
    <div v-else class="muted">No QPX views listed for this dataset.</div>
  </div>
</template>

<style scoped>
.loadbox {
  margin-top: 10px;
  border-top: 1px solid var(--border);
  padding-top: 10px;
}
.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
  font-size: 13px;
  margin-bottom: 8px;
}
.actions {
  display: flex;
  gap: 8px;
  flex-wrap: wrap;
}
.error {
  margin: 8px 0;
  font-size: 13px;
}
.all {
  font-weight: 600;
  font-size: 13px;
  display: flex;
  gap: 8px;
  align-items: center;
  padding-bottom: 6px;
  border-bottom: 1px solid var(--rule);
  margin-bottom: 4px;
}
.view-list {
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.view {
  display: flex;
  align-items: center;
  gap: 10px;
  font-size: 13px;
  cursor: pointer;
}
.view-name {
  flex: 1;
}
</style>
