<script setup lang="ts">
// Settings modal, in tabs: Appearance (theme + PTM colours), Data & download
// (heavy columns loaded by default, parallel download connections), Advanced
// (browse page size, …), Diagnostics (the app log).
import { ref } from "vue";
import {
  PTM_PALETTE,
  columnEnabledDefault,
  effectivePtmColor,
  resetPtmColor,
  DOWNLOAD_CONNECTIONS,
  setColumnEnabledDefault,
  setDownloadConnections,
  setPageSize,
  setPtmColor,
  setTheme,
  settings,
} from "../lib/settings";
import { OPTIONAL_COLUMNS } from "../lib/queries/table";
import DiagnosticsPanel from "./DiagnosticsPanel.vue";

defineProps<{ open: boolean }>();
const emit = defineEmits<{ (e: "close"): void }>();

const tab = ref<"appearance" | "data" | "advanced" | "diagnostics">("appearance");

function pick(id: string, e: Event) {
  const val = (e.target as HTMLInputElement).value;
  setPtmColor(id, val);
}

function pageSizeInput(e: Event) {
  setPageSize(Number((e.target as HTMLInputElement).value));
}

function connectionsInput(e: Event) {
  const input = e.target as HTMLInputElement;
  setDownloadConnections(Number(input.value));
  // Show the clamped value even when the stored one did not change.
  input.value = String(settings.downloadConnections);
}
</script>

<template>
  <Teleport to="body">
    <div v-if="open" class="overlay" @click.self="emit('close')">
      <div class="panel" role="dialog" aria-label="Settings">
        <div class="head">
          <h3>Settings</h3>
          <button class="icon" @click="emit('close')" title="Close">✕</button>
        </div>

        <nav class="tabs" aria-label="Settings sections">
          <button :class="{ active: tab === 'appearance' }" @click="tab = 'appearance'">Appearance</button>
          <button :class="{ active: tab === 'data' }" @click="tab = 'data'">Data &amp; download</button>
          <button :class="{ active: tab === 'advanced' }" @click="tab = 'advanced'">Advanced</button>
          <button :class="{ active: tab === 'diagnostics' }" @click="tab = 'diagnostics'">Diagnostics</button>
        </nav>

        <div class="body">
          <!-- Appearance: theme + PTM colours -->
          <template v-if="tab === 'appearance'">
            <section>
              <h4>Theme</h4>
              <div class="theme-row" role="radiogroup" aria-label="Theme">
                <button class="theme-btn" :class="{ on: settings.theme === 'light' }" @click="setTheme('light')">Light</button>
                <button class="theme-btn" :class="{ on: settings.theme === 'dark' }" @click="setTheme('dark')">Dark</button>
              </div>
              <p class="hint">
                Applies to the whole application background and text. PTM colours below apply to
                both themes.
              </p>
            </section>

            <section>
              <h4>PTM colours</h4>
              <p class="hint">
                Pick a colour for each modification family. Modified residues are tinted and underlined
                in it, and the badges in the modifications column carry it as a dot.
              </p>
              <div class="list">
                <div v-for="e in PTM_PALETTE" :key="e.id" class="ptm-row">
                  <span class="swatch" :style="{ background: effectivePtmColor(e.id) }"></span>
                  <span class="label">{{ e.label }}</span>
                  <input
                    type="color"
                    class="picker"
                    :value="effectivePtmColor(e.id)"
                    @input="pick(e.id, $event)"
                    :title="effectivePtmColor(e.id)"
                  />
                  <button
                    class="reset"
                    :disabled="!(e.id in settings.ptmColors)"
                    :title="`Reset ${e.label} to default`"
                    @click="resetPtmColor(e.id)"
                  >
                    ↺
                  </button>
                </div>
              </div>
            </section>
          </template>

          <!-- Data & download: heavy columns loaded by default -->
          <template v-else-if="tab === 'data'">
            <section>
              <h4>Columns loaded in Browse</h4>
              <p class="hint">
                Heavy or rarely used columns — peak/spectrum arrays, <span class="mono">cv_params</span> and the
                score/intensity lists — are <strong>not read by default</strong>, so for streamed datasets their
                data is never downloaded. Tick the ones to load by default; they are still available per view
                from the table's <em>Columns…</em> menu, and a single PSM's peaks always load on demand from the
                spectrum viewer.
              </p>
              <div class="list">
                <label v-for="o in OPTIONAL_COLUMNS" :key="o.col" class="col-row">
                  <input
                    type="checkbox"
                    :checked="columnEnabledDefault(o.col)"
                    @change="setColumnEnabledDefault(o.col, ($event.target as HTMLInputElement).checked)"
                  />
                  <span class="mono col-name">{{ o.col }}</span>
                  <span class="col-label">{{ o.label }}</span>
                </label>
              </div>
            </section>
            <section>
              <h4>Parallel download connections</h4>
              <p class="hint">
                <em>Download to cache</em> fetches each large file as several byte ranges at once. Remote
                storage limits the speed of each connection to a few MB/s, so more connections are usually
                faster, up to your own bandwidth. Lower it on a slow or shared network.
              </p>
              <div class="num-row">
                <input
                  type="number"
                  :min="DOWNLOAD_CONNECTIONS.min"
                  :max="DOWNLOAD_CONNECTIONS.max"
                  step="1"
                  class="num"
                  :value="settings.downloadConnections"
                  @change="connectionsInput"
                />
                <span class="hint-sm">connections ({{ DOWNLOAD_CONNECTIONS.min }}–{{ DOWNLOAD_CONNECTIONS.max }}, default {{ DOWNLOAD_CONNECTIONS.default }})</span>
              </div>
            </section>
          </template>

          <DiagnosticsPanel v-else-if="tab === 'diagnostics'" />

          <!-- Advanced: other configurable options -->
          <template v-else>
            <section>
              <h4>Browse page size</h4>
              <p class="hint">
                Rows shown per page in the Browse table. A larger page reads and renders more rows at
                once, which can be slower on big or streamed datasets.
              </p>
              <div class="num-row">
                <input type="number" min="20" max="2000" step="50" class="num" :value="settings.pageSize" @change="pageSizeInput" />
                <span class="hint-sm">rows per page (20–2000)</span>
              </div>
            </section>
          </template>
        </div>

        <div class="foot">
          <button class="primary" @click="emit('close')">Done</button>
        </div>
      </div>
    </div>
  </Teleport>
</template>

<style scoped>
.overlay {
  position: fixed;
  inset: 0;
  z-index: 1000;
  background: var(--scrim);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}
.panel {
  width: min(580px, 100%);
  max-height: 88vh;
  display: flex;
  flex-direction: column;
  background: var(--bg-panel);
  color: var(--text);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 20px 22px;
  box-shadow: var(--shadow);
}
.head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 8px;
}
.head h3 {
  margin: 0;
  font-size: 18px;
}
.head .icon {
  padding: 3px 9px;
  font-size: 13px;
  line-height: 1.2;
}
/* Underlined tab nav, as elsewhere in the app. */
.tabs {
  display: flex;
  gap: 4px;
  border-bottom: 1px solid var(--border);
  margin-bottom: 14px;
}
.tabs button {
  border: none;
  border-radius: 0;
  background: transparent;
  padding: 8px 14px 9px;
  color: var(--text-dim);
  border-bottom: 3px solid transparent;
  margin-bottom: -1px;
  font-size: 13px;
}
.tabs button:hover:not(:disabled) {
  color: var(--accent);
}
.tabs button.active {
  color: var(--accent);
  border-bottom-color: var(--accent);
  font-weight: 600;
}
.body {
  overflow: auto;
  flex: 1;
}
section {
  margin-bottom: 18px;
}
section + section {
  border-top: 1px solid var(--rule);
  padding-top: 14px;
}
h4 {
  margin: 0 0 10px;
  font-size: 14px;
  color: var(--text);
}
.hint {
  font-size: 12.5px;
  color: var(--muted);
  margin: 8px 0 12px;
  line-height: 1.5;
}
.hint-sm {
  font-size: 11.5px;
  color: var(--muted);
}
/* A two-way segmented control. */
.theme-row {
  display: inline-flex;
  border: 1px solid var(--border);
  border-radius: 6px;
  overflow: hidden;
}
.theme-btn {
  border: none;
  border-radius: 0;
  padding: 6px 22px;
  background: var(--bg-panel);
  color: var(--text-dim);
}
.theme-btn + .theme-btn {
  border-left: 1px solid var(--border);
}
.theme-btn.on {
  background: var(--accent);
  color: var(--on-accent);
}
.theme-btn.on:hover:not(:disabled) {
  color: var(--on-accent);
}
.list {
  display: flex;
  flex-direction: column;
  max-height: 340px;
  overflow: auto;
  border-top: 1px solid var(--rule);
}
.ptm-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 4px 4px;
  border-bottom: 1px solid var(--rule);
}
.swatch {
  width: 14px;
  height: 14px;
  border-radius: 3px;
  flex: none;
}
.label {
  flex: 1;
  font-size: 13px;
}
.picker {
  width: 40px;
  height: 26px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
}
.col-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 4px 4px;
  border-bottom: 1px solid var(--rule);
  font-size: 12.5px;
  cursor: pointer;
}
.col-row:last-child {
  border-bottom: none;
}
.col-row input {
  accent-color: var(--accent);
  cursor: pointer;
}
.col-name {
  font-size: 12px;
}
.col-label {
  color: var(--muted);
  margin-left: auto;
  text-align: right;
}
.reset {
  padding: 2px 8px;
  font-size: 13px;
}
.num-row {
  display: flex;
  align-items: center;
  gap: 10px;
}
.num {
  width: 120px;
  padding: 5px 8px;
  font-size: 13px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-panel);
  color: var(--text);
}
.foot {
  display: flex;
  justify-content: flex-end;
  padding-top: 14px;
  border-top: 1px solid var(--rule);
  margin-top: 12px;
}
</style>
