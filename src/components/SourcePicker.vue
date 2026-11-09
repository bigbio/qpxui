<script setup lang="ts">
import { ref } from "vue";
import { api } from "../lib/api";
import { loadDatasets } from "../store";
import S3Picker from "./S3Picker.vue";
import PortalPicker from "./PortalPicker.vue";
import DiscoverView from "./DiscoverView.vue";
import RecommendedDatasets from "./RecommendedDatasets.vue";

const busy = ref(false);
const error = ref("");
const activePanel = ref<"s3" | "portal" | "discover" | null>(null);

function togglePanel(panel: "s3" | "portal" | "discover") {
  activePanel.value = activePanel.value === panel ? null : panel;
}

async function run(fn: () => Promise<void>) {
  busy.value = true;
  error.value = "";
  try {
    await fn();
  } catch (e: any) {
    error.value = String(e);
  } finally {
    busy.value = false;
  }
}

async function openSingleFile() {
  await run(async () => {
    const path = await api.chooseFile();
    if (!path) return;
    const ds = await api.openFile(path);
    loadDatasets([ds], "single-file");
  });
}

async function openFolder() {
  await run(async () => {
    const path = await api.chooseFolder();
    if (!path) return;
    const scan = await api.scanCollection(path);
    loadDatasets(scan.datasets, "collection", scan.skipped);
  });
}
</script>

<template>
  <div class="picker">
    <section class="band">
      <div class="band-inner">
        <h1>Open a QPX dataset</h1>
        <p class="lead">
          QPX (Quantitative Proteomics eXchange) stores a dataset as a folder of
          <span class="mono">&lt;prefix&gt;.&lt;view&gt;.parquet</span> views (PSMs, features, protein
          groups, samples, …). Every query runs on this computer; remote files are read in place,
          only the parts a query needs.
        </p>

        <div class="sources">
          <button class="source" @click="openSingleFile" :disabled="busy">
            <span class="s-title">Single file</span>
            <span class="s-desc">One <span class="mono">*.parquet</span> view and the views next to it.</span>
          </button>
          <button class="source" @click="openFolder" :disabled="busy">
            <span class="s-title">Folder / collection</span>
            <span class="s-desc">A dataset folder, or a folder of many QPX projects.</span>
          </button>
          <button class="source" :class="{ on: activePanel === 's3' }" @click="togglePanel('s3')" :disabled="busy">
            <span class="s-title">S3 bucket</span>
            <span class="s-desc">Datasets in a bucket, read with your AWS credentials.</span>
          </button>
          <button class="source" :class="{ on: activePanel === 'portal' }" @click="togglePanel('portal')" :disabled="busy">
            <span class="s-title">Quantms portal</span>
            <span class="s-desc">A reanalysed dataset from portal.quantms.org.</span>
          </button>
          <button class="source" :class="{ on: activePanel === 'discover' }" @click="togglePanel('discover')" :disabled="busy">
            <span class="s-title">Discover quantms data</span>
            <span class="s-desc">Search the portal by text, organism, instrument, peptide or protein.</span>
          </button>
        </div>
      </div>
    </section>

    <div class="below">
      <div class="error pad" v-if="error">{{ error }}</div>
      <div v-if="busy" class="muted">Working… <span class="spinner"></span></div>

      <S3Picker v-if="activePanel === 's3'" />
      <PortalPicker v-else-if="activePanel === 'portal'" />
      <DiscoverView v-else-if="activePanel === 'discover'" />

      <RecommendedDatasets v-else />
    </div>
  </div>
</template>

<style scoped>
/* A flex child of the app body: fill it and scroll on its own. */
.picker {
  flex: 1;
  min-width: 0;
  overflow: auto;
}
/* The deep-blue opening band, as on portal.quantms.org. */
.band {
  background: var(--band);
  color: var(--on-dark);
}
.band-inner,
.below {
  max-width: 1440px;
  margin: 0 auto;
  padding: 0 32px;
}
.band-inner {
  padding-top: 34px;
  padding-bottom: 34px;
}
h1 {
  margin: 0 0 8px;
  font-size: 30px;
  line-height: 1.2;
  color: #ffffff;
}
.lead {
  margin: 0;
  max-width: 820px;
  font-size: 15px;
  line-height: 1.6;
  color: var(--on-dark);
}
.lead .mono {
  font-size: 13.5px;
}
.sources {
  display: grid;
  grid-template-columns: repeat(5, minmax(0, 1fr));
  gap: 12px;
  margin-top: 24px;
}
@media (max-width: 1100px) {
  .sources {
    grid-template-columns: repeat(3, minmax(0, 1fr));
  }
}
@media (max-width: 640px) {
  .sources {
    grid-template-columns: 1fr;
  }
}
.source {
  display: flex;
  flex-direction: column;
  gap: 4px;
  text-align: left;
  padding: 14px 16px;
  background: var(--bg-panel);
  border: 1px solid transparent;
  border-radius: 6px;
  color: var(--text);
}
/* On the dark band the cards need an edge to stand off it. */
:global(:root[data-theme="dark"]) .source {
  border-color: var(--border);
}
.source:hover:not(:disabled),
.source.on {
  border-color: var(--accent);
  color: var(--text);
  box-shadow: inset 0 -3px 0 var(--accent);
}
.s-title {
  font-weight: 600;
  font-size: 15px;
}
.s-desc {
  color: var(--muted);
  font-size: 13px;
  font-weight: 400;
  line-height: 1.4;
}
.below {
  padding-top: 8px;
  padding-bottom: 36px;
}
.below > :deep(*:first-child) {
  margin-top: 24px;
}
</style>
