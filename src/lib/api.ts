import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { settings } from "./settings";

export interface ExportFilter {
  name: string;
  extensions: string[];
}

export interface ViewInfo {
  name: string;
  view: string;
  format: string;
  path: string;
  sizeBytes: number | null;
}

export interface DatasetInfo {
  prefix: string;
  path: string;
  views: ViewInfo[];
}

export interface SkippedFolder {
  path: string;
  reason: string;
}

export interface CollectionScan {
  datasets: DatasetInfo[];
  skipped: SkippedFolder[];
}

export interface S3Object {
  key: string;
  sizeBytes: number | null;
  lastModified: string | null;
}

export interface S3Listing {
  bucket: string;
  prefix: string;
  objects: S3Object[];
  datasets: DatasetInfo[];
}

export interface PortalDataset {
  accession: string;
  title?: string | null;
  organism?: string | null;
  instrument?: string | null;
  samples?: number | null;
  proteins?: number | null;
  peptides?: number | null;
  psmCount?: number | null;
  totalSize?: number | null;
  datasetRef?: string | null;
  pxUrl?: string | null;
  collectionTitle?: string | null;
  description?: string | null;
  keywords?: string[] | null;
  tissues?: string[] | null;
  publication?: { doi?: string | null } | null;
  source?: string | null;
}

/** The key of a portal dataset: some accessions hold several runs (one per
 * organism), so it is the run's ref. Without one it is the bare accession,
 * which the backend resolves by scanning the run sub-folders. */
export function portalRef(d: PortalDataset): string {
  return d.datasetRef ?? d.accession;
}

export interface PortalSearchResult {
  datasets: PortalDataset[];
  total: number;
  page: number;
  totalPages: number;
  sort: string;
}

export interface CollectionOption {
  name: string;
  title?: string | null;
}

export interface PortalFacets {
  organisms: string[];
  instruments: string[];
  collections: CollectionOption[];
}

export interface PsHit {
  datasetAccession: string;
  datasetRef: string;
  collection?: string | null;
  organism?: string | null;
  organismPart?: string | null;
  instrument?: string | null;
  nPeptidoforms: number;
  nPeptides: number;
  totalObs: number;
  samplePeptidoforms: string[];
}

export interface PsSearchResult {
  totalDatasets: number;
  datasets: PsHit[];
  unmapped: boolean;
}

export interface PsModification {
  name: string;
  nDatasets: number;
}

export interface StreamHandle {
  url: string;
  sizeBytes: number;
}

/** Payload of the backend's `download-progress` event, one per transferred chunk. */
export interface DownloadProgress {
  /** The download's `cancelId`. */
  download: string;
  file: string;
  done: number;
  /** Total bytes, or 0 when unknown. */
  total: number;
  percent: number;
  phase: "downloading" | "done";
}

export interface StreamStat {
  sizeBytes: number;
  fetchedBytes: number;
  requests: number;
}

export const api = {
  /** Native file picker for a single QPX file. */
  chooseFile: (): Promise<string | null> =>
    open({
      multiple: false,
      filters: [{ name: "QPX", extensions: ["parquet"] }],
    }) as Promise<string | null>,

  /** Native folder picker. */
  chooseFolder: (): Promise<string | null> =>
    open({ directory: true, multiple: false }) as Promise<string | null>,

  openFile: (path: string) => invoke<DatasetInfo>("open_file", { path }),

  scanCollection: (path: string) => invoke<CollectionScan>("scan_collection", { path }),

  listS3: (bucket: string, prefix?: string) =>
    invoke<S3Listing>("list_s3", { bucket, prefix }),

  cacheS3Views: (bucket: string, paths: string[], cancelId: string) =>
    invoke<ViewInfo[]>("cache_s3_views", { bucket, paths, cancelId, connections: settings.downloadConnections }),

  /** Register S3 views with the backend range proxy (lazy, no download). */
  streamS3Urls: (bucket: string, paths: string[]) =>
    invoke<Record<string, StreamHandle>>("stream_s3_urls", { bucket, paths }),
  /** Register public portal (https) files with the backend range proxy. */
  streamHttpUrls: (urls: string[]) =>
    invoke<Record<string, StreamHandle>>("stream_http_urls", { urls }),
  /** Register local parquet files with the range proxy (lazy reads from disk). */
  streamLocalFiles: (paths: string[]) =>
    invoke<Record<string, StreamHandle>>("stream_local_files", { paths }),
  /** Bytes actually fetched per streamed view (keyed by its s3:// or https path). */
  streamS3Stats: () => invoke<Record<string, StreamStat>>("stream_s3_stats"),

  /** Ask the backend to abort an in-flight download batch. */
  cancelDownload: (id: string) => invoke<void>("cancel_download", { id }),
  /** Free bytes on the download cache disk, and the spare amount always kept free. */
  cacheFreeSpace: (kind: "portal" | "s3") =>
    invoke<{ free: number; reserve: number }>("cache_free_space", { kind }),

  /** Rust chooses the path in a native save dialog and writes the export. */
  saveExport: (defaultName: string, filters: ExportFilter[], base64: string) =>
    invoke<string | null>("save_export", { defaultName, filters, base64 }),

  // ---- quantms portal (public example source over HTTPS) ----
  portalRegistry: (pageSize?: number) =>
    invoke<PortalDataset[]>("portal_registry", { pageSize }),
  /** Server-side search over the quantms portal catalogue (q text + facets + sort). */
  portalSearch: (
    query: string | null,
    opts: {
      collection?: string | null;
      organism?: string | null;
      instrument?: string | null;
      sort?: string | null;
      page?: number;
      pageSize?: number;
    }
  ) => invoke<PortalSearchResult>("portal_search", { query, ...opts }),
  /** Organism / instrument / collection facet options for the discover UI. */
  portalFacets: () => invoke<PortalFacets>("portal_facets"),
  /** Cross-dataset peptide / protein / gene search over the quantms portal. */
  psSearch: (
    query: string,
    opts: {
      dtype: string | null;
      matchMode: string | null;
      organism: string | null;
      modification: string | null;
      limit: number;
    }
  ) => invoke<PsSearchResult>("ps_search", { query, ...opts }),
  psModifications: () => invoke<PsModification[]>("ps_modifications"),
  portalListViews: (dataset: string, organism?: string | null, psmCount?: number | null) =>
    invoke<ViewInfo[]>("portal_list_views", { dataset, organism, psmCount }),
  portalCacheViews: (urls: string[], cancelId: string) =>
    invoke<ViewInfo[]>("portal_cache_views", { urls, cancelId, connections: settings.downloadConnections }),
};
