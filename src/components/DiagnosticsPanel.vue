<script setup lang="ts">
// Settings → Diagnostics: the app log, to look at and to share with the
// developers when something is slow or fails.
import { computed, onMounted, ref } from "vue";
import { readLog, setDetailedLogging, type LogInfo } from "../lib/log";
import { saveText } from "../lib/export";
import { setDetailedLog, settings } from "../lib/settings";

/** Lines shown in the preview (the saved and copied log has all of it). */
const PREVIEW_LINES = 300;

const info = ref<LogInfo | null>(null);
const error = ref("");
const note = ref("");

const preview = computed(() => {
  const lines = (info.value?.text ?? "").trimEnd().split("\n");
  return lines.slice(-PREVIEW_LINES).join("\n");
});

async function refresh() {
  error.value = "";
  try {
    info.value = await readLog();
  } catch (e: any) {
    error.value = String(e?.message ?? e);
  }
}

async function toggleDetailed(e: Event) {
  const on = (e.target as HTMLInputElement).checked;
  setDetailedLog(on);
  try {
    await setDetailedLogging(on);
  } catch (err: any) {
    error.value = String(err?.message ?? err);
  }
  await refresh();
}

/** The log with a header naming the app, for a bug report. */
function report(): string {
  const head = [
    `QPX UI diagnostics, ${new Date().toISOString()}`,
    `User agent: ${navigator.userAgent}`,
    `Detailed logging: ${info.value?.detailed ? "on" : "off"}`,
    `Log file: ${info.value?.path ?? "none"}`,
    "",
  ].join("\n");
  return head + (info.value?.text ?? "");
}

async function copy() {
  await refresh();
  try {
    await navigator.clipboard.writeText(report());
    note.value = "Log copied to the clipboard.";
  } catch (e: any) {
    error.value = `Copy failed: ${e?.message ?? e}`;
  }
}

async function save() {
  await refresh();
  try {
    const path = await saveText(`qpxui-log-${new Date().toISOString().slice(0, 10)}`, report(), "log");
    if (path) note.value = `Saved to ${path}`;
  } catch (e: any) {
    error.value = String(e?.message ?? e);
  }
}

async function copyPath() {
  if (!info.value?.path) return;
  await navigator.clipboard.writeText(info.value.path).catch(() => {});
  note.value = "Path copied.";
}

onMounted(refresh);
</script>

<template>
  <section>
    <h4>Log</h4>
    <p class="hint">
      The app writes what it does to a log file: datasets opened, downloads, slow queries and errors. When something
      is slow or fails, save or copy the log and attach it to your report to the developers.
    </p>
    <label class="check">
      <input type="checkbox" :checked="settings.detailedLog" @change="toggleDetailed" />
      <span>
        <strong>Detailed logging</strong>: also every query and every streamed range, with timings. Turn it on, repeat
        what was slow, then save the log.
      </span>
    </label>
    <div class="path">
      <span class="hint-sm">Log file:</span>
      <span class="mono">{{ info?.path ?? "not available" }}</span>
      <button v-if="info?.path" class="xs" @click="copyPath">Copy path</button>
    </div>
    <div class="actions">
      <button @click="save">Save log…</button>
      <button @click="copy">Copy log</button>
      <button @click="refresh">Refresh</button>
      <span v-if="note" class="hint-sm">{{ note }}</span>
    </div>
    <div v-if="error" class="error">{{ error }}</div>
    <pre class="log mono">{{ preview || "The log is empty." }}</pre>
  </section>
</template>

<style scoped>
.hint {
  font-size: 13px;
  color: var(--muted);
  margin: 0 0 10px;
}
.hint-sm {
  font-size: 12px;
  color: var(--muted);
}
.check {
  display: flex;
  gap: 8px;
  align-items: flex-start;
  font-size: 13px;
  margin-bottom: 10px;
}
.path {
  display: flex;
  gap: 8px;
  align-items: baseline;
  font-size: 12px;
  margin-bottom: 10px;
  word-break: break-all;
}
.actions {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: wrap;
  margin-bottom: 10px;
}
.log {
  font-size: 11px;
  line-height: 1.45;
  max-height: 300px;
  overflow: auto;
  white-space: pre-wrap;
  word-break: break-all;
  background: var(--bg-alt);
  border: 1px solid var(--border);
  border-radius: 4px;
  padding: 8px;
  margin: 0;
}
.error {
  font-size: 13px;
  margin-bottom: 8px;
}
</style>
