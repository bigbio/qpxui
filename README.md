# QPX UI

[![CI](https://github.com/bigbio/qpxui/actions/workflows/ci.yml/badge.svg)](https://github.com/bigbio/qpxui/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/bigbio/qpxui?include_prereleases)](https://github.com/bigbio/qpxui/releases/latest)
[![License](https://img.shields.io/badge/license-Apache--2.0-blue)](LICENSE)

A **desktop viewer for [QPX](https://github.com/bigbio/qpx) (Quantitative Proteomics eXchange)
data**. You can open a local file or folder, an S3 bucket, or any reanalysed dataset on the
[quantms portal](https://portal.quantms.org). Each dataset gets a report with QC charts and
statistics, plus protein and peptide search, a data browser and a SQL console.

QPX datasets are folders of `<prefix>.<view>.parquet` files (`PXD014414.feature.parquet`,
`PXD014414.pg.parquet`, `PXD014414.run.parquet`, …). QPX UI runs [DuckDB](https://duckdb.org)
inside the app, so every query runs on your machine. Remote files are **streamed lazily**: only
the parquet footer and the column chunks a query needs are fetched, never the whole file.

Built with [Tauri v2](https://v2.tauri.app/) (Rust), Vue 3 + TypeScript + Vite, `duckdb-wasm` and ECharts.

![Dataset report for PXD014414: headline counts and identification QC](docs/screenshots/report-overview.png)

---

## Contents

- [Install](#install)
- [Quick start](#quick-start)
- [Features](#features)
  - [Data sources](#data-sources)
  - [Dataset report](#dataset-report)
  - [Quantification](#quantification)
  - [Protein & peptide search](#protein--peptide-search)
  - [Spectrum viewer](#spectrum-viewer)
  - [Browse, summary & SQL](#browse-summary--sql)
  - [PTMs, settings & export](#ptms-settings--export)
- [Streaming from S3 and the portal](#streaming-from-s3-and-the-portal)
- [Development](#development)
- [Architecture](#architecture)
- [Releasing](#releasing)
- [Limitations](#limitations)
- [License](#license)

---

## Install

Download the installer for your platform from the
[latest release](https://github.com/bigbio/qpxui/releases/latest):

| Platform | File | Notes |
|----------|------|-------|
| macOS, Apple Silicon | `qpxui_<version>_aarch64.dmg` | Open the `.dmg` and drag **qpxui** to Applications. |
| macOS, Intel | `qpxui_<version>_x64.dmg` | Same as above. |
| Windows 10/11 | `qpxui_<version>_x64-setup.exe` or `.msi` | Uses the WebView2 runtime that ships with Windows. |
| Linux | `.AppImage`, `.deb`, `.rpm` | `chmod +x qpxui*.AppImage && ./qpxui*.AppImage`, or install the package. |

> **macOS: the app is not notarized yet.** The first launch may say the app "is damaged" or
> "can't be opened". Right-click the app → **Open**, or run
> `xattr -dr com.apple.quarantine /Applications/qpxui.app`.

---

## Quick start

1. Launch QPX UI and pick a source:
   - **Quantms portal**: pick a reanalysed dataset (for example `PXD004452`), then press **Stream**.
   - **Discover quantms data**: search the portal's datasets, or find the datasets that contain a
     peptide, protein or gene; expand one to stream or download its views. The start page also
     lists a few recommended datasets.
   - **Folder / collection**: a folder with one dataset, or with many `PXD*/` dataset folders.
   - **Single file**: one `*.parquet` view; the files next to it are picked up too.
   - **S3 bucket**: a bucket/prefix, read with your AWS credentials.
2. The **dataset report** opens. Browse the tabs, or type a protein, gene or peptide into the
   search box in the sidebar.
3. Click any view in the sidebar (`feature`, `psm`, `pg`, …) to browse its rows, chart its columns,
   or query it with SQL.

---

## Features

### Data sources

| Source | What it does |
|--------|--------------|
| **Single file** | Opens one `*.parquet` view plus its sibling views from the same folder. |
| **Folder / collection** | Opens one QPX dataset folder, or a folder tree holding many projects. Subfolders that cannot be read are skipped and listed above the datasets in the sidebar. |
| **S3 bucket** | Lists datasets under a bucket/prefix using your AWS credentials. You can stream the files lazily or download them to a local cache. S3-compatible stores (MinIO, Ceph) work too. |
| **Quantms portal** | Searchable list of the datasets on [portal.quantms.org](https://portal.quantms.org) (title, organism, instrument, quantification/acquisition types, counts). Filter by an instrument class or model, quantification family, and acquisition method. You can stream a dataset lazily or download it to a local cache. No account needed. |
| **Discover quantms data** | Searches the portal's datasets by text, organism, instrument and collection, and finds the datasets that contain a peptide, protein or gene. In **Auto** mode, an upper-case string of 8 or more standard amino acids is searched as a peptide, as in the sidebar search box, and anything else as a protein or gene. Expand a result to stream or download its views. |

The instrument picker is a searchable PSI-MS hierarchy: select an architecture class
to include its descendant models, a vendor/series group, or an individual model.
The app preserves the ontology's actual multiple-parent relationships rather than
guessing from instrument names. It fetches PSI-MS 4.2.15 and caches the hierarchy for
the app session. Unrecognized portal models remain individually selectable; if the
hierarchy is unavailable, an error notice appears and the individual models remain.

Quantification and acquisition metadata load in the background from the small QPX
run view, without scanning feature or PSM data, and are cached by dataset version.
Acquisition is the declared `acquisition_method`; quantification family (Label-free,
TMT, iTRAQ or SILAC) is derived from declared sample labels, available in the tooltip.
All runs contribute: mixed methods match each applicable filter. Missing/unrecognized
values stay **Unknown**, never assumed label-free; unavailable versions show a notice.
Browsing remains available while method filters load. Discover combines the filters
before pagination so counts include the complete result set. Cards show one source-named
repository link when a URL is available, rather than a duplicate plain source tag.

Downloads show progress and can be cancelled at any point, even in the middle of a file; partial
files are removed. Before a download starts, the app checks the free space on the cache disk: it
refuses a download that would leave less than 1 GB free, and asks before downloading more than
2 GB. A size that the portal listing or S3 does not report is shown as —; such a file counts as
0 in these checks, and a selection that includes one shows its total as —. Choosing
**New source** or another dataset cancels a running download and stops the previous dataset's
queries.

While you stream, the sidebar shows how much of each file has actually been fetched (for example
`16 MB / 170 MB`).

### Dataset report

Selecting a dataset that has a `feature`, `psm` or `pg` view opens its report. It works like the
portal's Dataset Explorer, but runs locally on any QPX data. You can also reach it from
**Report & search** in the sidebar.

- **Header**: title, description, organisms, instruments, enzymes, software, and PRIDE/PubMed
  links. **Cite this data** copies a citation or saves it as BibTeX or RIS.
- **Headline counts**: protein groups, peptides, features, PSMs, runs and samples.

A count or header query that fails shows its error in place; the rest of the report still loads.

The report has these tabs. A tab only appears when the dataset has the data for it:

| Tab | Contents |
|-----|----------|
| **Overview** | Identifications per run. Distributions of precursor charge, peptide length, missed cleavages, peptides per protein group, precursor mass error (ppm), retention time, intensity, posterior error probability and peptide q-value. |
| **QC** | Identifications per run (features, peptides, proteins, protein groups). Median intensity and median mass error per run, to spot loading or calibration drift. Intensity distribution per sample (TMT channels are matched to samples through the run view). A per-run table with sample, fraction and instrument. |
| **PTMs** | How often each modification occurs (counted by peptidoforms, peptides or features), drawn in your PTM colours. Modified sites by residue, modifications per peptide, and a table; click a modification to list its peptides. |
| **Samples** | Charts of the sample metadata (organism, tissue, disease, cell line, sex, …), coloured by biological group (see below), and a filterable sample table showing which runs and channels each sample was measured in. |
| **Proteins / Peptidoforms** | Search; see below. |
| **Quantification** | Protein × sample abundance matrix, sample correlation, PCA and per-protein profiles; see below. |

Statistics are computed over target rows only (decoys left out). Histograms show the central 99%
of values, and the mass-error histogram also drops extreme outliers such as isotope-error matches.
Continuous distributions are drawn as a filled line and counts as bars; points and bars are
labelled with the start of their bin, and hovering one shows its range.
Results are cached, so switching tabs is instant.

![QC tab: identifications, median intensity and mass error per run, intensity per sample](docs/screenshots/report-qc.png)

### Quantification

The **Quantification** tab appears when the `pg` view (or, failing that, the `feature` view) has
`intensities`. It builds a protein × sample matrix of log2 abundances:

- **Source**: protein-group intensities from `pg`, or feature intensities summed per anchor protein.
- **Columns**: run channels are mapped to samples through the run view's `samples` (TMT/iTRAQ
  channels are matched by label, fractions are summed); otherwise there is one column per run.
- **Options**: median normalisation, include/exclude contaminants, the minimum fraction of samples
  a protein must be quantified in, and **colour by** a sample attribute (disease, tissue, …).
- **Charts**: abundance distribution and proteins quantified per sample; a Pearson **sample
  correlation** heatmap ordered by average-linkage clustering; **PCA** with selectable components and
  variance explained.
- **Abundance profile**: pick a protein from the table to see its abundance in every sample,
  coloured by group, with group means.
- **Colours**: samples are coloured as on the portal. Tissues and cell types take the colour of
  their organ system (looked up in UBERON through the EBI Ontology Lookup Service), organisms of
  their taxonomic group (UniProt taxonomy), and diseases of their class (healthy, cancer, other);
  the values of one group take lighter and darker shades of its colour. Developmental stages
  take a ramp from youngest to oldest, and other attributes the palette by sample count. The
  lookups are kept between sessions; without a connection, values take the palette by count.
- **Export**: every chart as PNG/CSV, and the whole log2 matrix as CSV (**Matrix CSV**).

![Quantification tab: sample correlation, PCA coloured by disease and a protein abundance profile](docs/screenshots/quantification.png)

### Protein & peptide search

- **Proteins**: search by accession, protein name or gene name (for example `P04637`, `ALBU`, `EGFR`).
  The search runs on the protein-group (`pg`) view when there is one, and otherwise on the
  proteins in the feature or PSM view. Clicking a protein shows:
  - UniProt links, peptidoform, peptide and feature counts;
  - a per-run evidence chart;
  - its PTM-coloured peptidoforms;
  - its protein group in each run.
- **Peptidoforms**: search by sequence (*contains*, *starts with* or *exact*), or by peptidoform text
  such as `M[Oxidation]` or `UNIMOD:21`. Filters for modification and minimum charge are available.
  Clicking a peptidoform opens its **Profile**: its intensity in every sample (runs and TMT channels
  mapped to samples through the run view), the distribution per sample group, and its retention time
  in each run. The **List** pane shows every observation (run, charge, RT, m/z, ppm, PEP, intensity),
  and its protein links open the protein.

![Protein view: per-run evidence, PTM-coloured peptidoforms and the protein group in each run](docs/screenshots/protein.png)

- The **sidebar search box** searches the selected dataset. An upper-case amino-acid string of 8 or
  more residues is searched as a peptide.

### Spectrum viewer

When the PSM view stores fragment peaks (`mz_array` / `intensity_array`, as in the
[π-MSNet](https://github.com/PHOENIXcenter/pi-MSnet) collection), the peptide evidence pane lists
PSMs, and clicking one opens an annotated spectrum in the style of the PRIDE USI viewer:

- stick plot with b (blue) and y (red) ions, neutral losses and labels; zoom with the wheel or slider;
- fragment coverage map, mass-error panel and a b/y fragment ion table;
- annotations either **from the file** (spectrum_utils labels stored in `ion_type_array`) or
  **computed** here, with an adjustable tolerance (ppm or Da), fragment charge and H₂O/NH₃ losses;
- the PSM's USI (`mzspec:PXD…:run:scan:N:peptidoform/z`), with a link to the PRIDE USI viewer;
- **mirror spectra** to check an identification against a reference drawn upside down:
  - another PSM of the same peptidoform;
  - a predicted spectrum from [Koina](https://koina.wilhelmlab.org) (Prosit 2020 HCD,
    AlphaPeptDeep, MS²PIP; the model is picked from the peptidoform's modifications);
  - any spectrum by USI, fetched from PRIDE PROXI (ProteomeCentral as fallback);
  - the theoretical b/y ladder.

  Matched peaks are scored with the cosine of square-root intensities and the normalised spectral
  angle;
- PNG and CSV (peaks + annotations) export.

![Spectrum viewer: annotated PSM mirrored against a Prosit prediction (cosine 0.71)](docs/screenshots/spectrum-mirror.png)

Predictions and USI lookups need internet access. Annotation and scoring run locally;
reading a streamed PSM also needs access to its remote source.

Only the selected PSM's row is read, so this also works on large streamed files. Whether a file
has spectra is decided from the parquet footer statistics. π-MSNet file names
(`<prefix>-MSNet.parquet`, `<prefix>-MSNet.psm.parquet`) are recognised as the PSM view of `<prefix>`.

### Browse, summary & SQL

Click a view in the sidebar to work with its rows directly:

- **Browse**: a table showing 500 rows per page, with column sorting and a live SQL `WHERE` filter;
  the row count follows the filter.
  Nested cells (lists and structs) can be expanded.
- **Summary**: automatic charts per column, and a histogram of any numeric column. It has up to 30
  bins (whole-number bins for integer columns) labelled with their start value, and leaves out
  infinite and NaN values. Hover a bar for its range and count.
- **Query**: SQL (DuckDB dialect, the same engine as `qpxc query sql`) against the registered
  views, with example queries to start from.

### PTMs, settings & export

- Peptides and peptidoforms are colour-coded by modification: a modified residue is tinted and
  underlined in its modification's colour, with a short code and a tooltip (name, CV accession,
  residue, position). A modification takes the colour of its CV accession, or else of the longest
  known keyword in its name, so *Dimethyl* is not coloured as *Methyl*. The default colours follow
  the portal: biological modifications (phosphorylation, oxidation, acetylation, …) take distinct
  colours, and fixed or chemical ones such as carbamidomethylation are grey.
- **Settings**: light or dark theme, and a colour for each PTM, which every table and chart uses.
- **Export**: every chart can be saved as PNG or CSV, and every table as CSV, through the system
  save dialog.

---

## Streaming from S3 and the portal

**Stream** queries files where they are stored, without downloading them:

- The Rust backend runs a local HTTP range proxy on `127.0.0.1` (`src-tauri/src/commands/stream/`).
- DuckDB reads only the parquet footer plus the column chunks and row groups each query touches.
- The proxy fetches just those byte ranges, in 4 MB requests, 8 in parallel. It keeps them in a
  512 MB in-memory cache of 256 KB blocks, so repeated queries don't refetch.
- **S3**: requests are signed with the default AWS credential chain (environment variables,
  `~/.aws/credentials`, SSO profile or IAM role). Credentials never reach the webview, and buckets
  need no CORS setup.

  ```bash
  AWS_PROFILE=myprofile AWS_REGION=eu-west-1 open -a qpxui    # macOS
  AWS_ENDPOINT_URL=http://localhost:9000 qpxui                # MinIO / S3-compatible
  ```
- **Portal**: the proxy follows the portal's redirect to a short-lived signed storage URL, and gets
  a new one when it expires.

Registry and filter requests to the portal API are retried twice, after 0.5 s and then 1 s, when
the connection fails or the server answers with a 5xx error. Other errors, such as a 404 or an
invalid reply, are reported at once.

Portal HTTPS requests use the operating system's certificate verifier and trust store, including
system-provided intermediates on macOS and Windows. If your company installs its CA in the system
store, no extra configuration should be needed.

Alternatively, set `SSL_CERT_FILE` to a PEM certificate bundle and/or `SSL_CERT_DIR` to directories
of PEM certificates before starting qpxui. As in uv, non-empty settings replace the default trust
source: include every CA needed for portal and storage hosts. Separate multiple directories with
`:` on Unix or `;` on Windows. Configured paths that cannot be read, or configured sources that
collectively contain no certificates, cause an error rather than falling back to system trust.

```bash
SSL_CERT_FILE=/path/to/ca-bundle.pem qpxui
```

On macOS, launch the executable directly (for example,
`SSL_CERT_FILE=/path/to/ca-bundle.pem /Applications/qpxui.app/Contents/MacOS/qpxui`);
on Windows PowerShell, set `$env:SSL_CERT_FILE` before launching the executable.
Only use certificates from a trusted source. Restart the app after changing the configuration.
These settings apply to portal registry, search, listing, downloads and streaming; the separate S3 client
may handle certificate configuration differently. HTTPS certificate verification remains enabled.

**Download to cache** instead fetches whole files once (with progress and cancel), for fully
offline use. Cached files are kept in the OS cache folder (`~/Library/Caches/qpxui/` on macOS,
`~/.cache/qpxui/` on Linux, `%LOCALAPPDATA%\qpxui\` on Windows). S3 downloads go to its `s3-cache/`
folder. The first S3 download after the app starts deletes the files that older versions left
there in their bucket/key layout. If one cannot be deleted, that download fails with its path and
the reason, and the next S3 download tries again.

The engine, workers and signed Parquet extensions are bundled with the app. Local and cached
Parquet files can be queried on a first launch without an internet connection. Portal discovery,
remote streaming, Koina predictions, USI retrieval and the organ-system and taxon lookups behind
the sample colours still need a connection.

### Large streamed files

Opening a streamed file of 1 GB or more reads only its parquet footer, so it opens in seconds
however big it is. Anything that would read whole columns waits for you to ask:

- **Header**: PSM and feature counts come from the footer; protein, peptide and run counts show a
  **count** button.
- **Overview, QC, PTMs, Quantification**: a **Compute** button shows a lower-bound estimate of the
  data it will fetch.
- **Proteins and Peptidoforms**: wait for a search term; peptide search defaults to an exact sequence match.
- **Browse**: peak arrays start hidden; **Summary** builds its charts on request.

How fast searches and spectra are depends on how the file was written: files sorted by peptidoform
with small row groups (`qpxc transform relayout`) let each lookup read a few MB.

---

## Development

**Prerequisites**

- Node.js 20+ and npm
- Rust (stable) via [rustup](https://rustup.rs)
- Tauri system dependencies:
  - macOS: Xcode Command Line Tools (`xcode-select --install`);
  - Windows: [WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) and MSVC build tools;
  - Linux: `libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev libssl-dev patchelf`
    (see the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)).

**Run and build**

```bash
npm install
npm run tauri dev      # run the app with hot reload (also compiles the Rust backend)
npm run tauri build    # installers for this platform → src-tauri/target/release/bundle/
```

`npm run dev` and `npm run build` first run `scripts/fetch-duckdb-extensions.mjs`; Tauri's
dev/build hooks and the CI frontend build use these scripts too. The first build downloads the
signed Parquet extension for both WASM bundles into the ignored `public/duckdb-extensions/`
directory. Later builds verify SHA-256 and reuse these files without a download. A mismatched
hash fails the build; remove the affected file before downloading it again. The application
loads extensions from its own assets, with signature verification enabled and no online fallback.

When upgrading `@duckdb/duckdb-wasm`, update `VERSION` and both `SHA256` entries in that script
from the matching official Parquet extension downloads. The script reads the embedded engine's
version through the installed package's Node bindings and fails if it differs from the pin.

**Checks** (the same ones CI runs on every PR):

```bash
npm run lint                   # ESLint
npm run typecheck              # vue-tsc
npm test                       # Vitest unit tests (src/lib/**/*.test.ts)
npm run build                  # vite build; the backend embeds dist/
cd src-tauri
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                     # add -- --ignored for the tests that need the portal
```

CI also builds the installers for Linux, Windows and macOS (Apple Silicon and Intel), as the
release does, and keeps them as artifacts of the workflow run.

**Sample data**

```bash
node scripts/fetch-sample.mjs   # small example feature/psm views → ./sample
pip install "qpx[all]"          # or convert your own search results to QPX
qpxc convert diann --report-path report.tsv --sdrf-file data.sdrf.tsv --output-folder ./sample/PXD000000
```

Many reanalysed datasets (e.g. `PXD004452`) can be opened directly with the **Quantms portal** source.

---

## Architecture

```
┌───────────────────────────────────────────────────────────────────┐
│ WebView: Vue 3 + duckdb-wasm + ECharts                            │
│  SourcePicker · PortalPicker · S3Picker · DiscoverView            │
│  DatasetExplorer (sidebar, search box)                            │
│  ReportView ─ Overview · QC · PTMs · Samples · Proteins ·         │
│               Quantification · Peptidoforms · SpectrumViewer      │
│  ViewPanel  ─ Browse · Summary · Query                            │
│        all SQL runs here, in DuckDB (wasm)                        │
└──────────────┬─────────────────────────────────┬──────────────────┘
               │ Tauri IPC                        │ HTTP range reads
┌──────────────▼─────────────────────────────────▼──────────────────┐
│ Rust backend                                                      │
│  sources.rs     QPX discovery (file / folder / collection)        │
│  io.rs          native save dialog and export writes              │
│  s3.rs          list / download S3 objects (aws-sdk-s3, rustls)   │
│  portal/        catalogue.rs  registry, search, facets            │
│                 listing.rs    a dataset's runs and views          │
│                 download.rs   views into the download cache       │
│  peptide_search.rs  datasets holding a peptide, protein or gene   │
│  portal_http.rs shared HTTPS client, certificate settings         │
│  stream/        127.0.0.1 range proxy: S3, portal, local files    │
│                 register.rs   files to stream, commands, stats    │
│                 server.rs     DuckDB's range requests             │
│                 fetch.rs      source reads, block cache           │
│  transfers.rs   download loop, cache folders, free space, cancel  │
│  progress.rs    download progress events                          │
└───────────────────────────────────────────────────────────────────┘
```

The front end has three layers, kept apart by ESLint import rules (`eslint.config.js`):

- `src/lib/` calls the backend (`api.ts`, `progress.ts`, `downloads.ts`), runs DuckDB
  (`duck.ts`), builds every SQL query (`queries/`, one module per tab or view, on the shared
  helpers in `report.ts`) and holds the charts and pure helpers. It imports neither the store
  nor components.
- `src/store.ts` holds the app state and imports only `lib`.
- Components render and handle input; they reach the backend and run SQL only through `lib`.

The release WebView CSP allows bundled assets, WASM, local DuckDB workers, Tauri IPC and the
Rust range proxy on `127.0.0.1` with a dynamic port. Images can use `data:`/`blob:` and chart
tooltips can use inline style attributes; scripts cannot use `unsafe-eval` or inline handlers.
Charts load the bundled `vue-echarts/style.css` rather than inline styles, and the IBM Plex fonts
are bundled too. The only
allowed external connections are the Koina and two PROXI/USI endpoints used by spectrum mirrors,
and the UniProt taxonomy search and EBI OLS ontology endpoints used to colour samples by
biological group. Portal and S3 requests run in Rust
and do not need external WebView permissions. CSP supplements the native IPC capabilities.
Development loads Vite directly at `http://localhost:1420`, which does not serve this CSP;
verify the policy in a release build.

Exports go through `api.saveExport` to the Rust `save_export` command, which opens the native
save dialog and writes only to its selected destination. Cancelling returns `null` without a
write. The frontend has no arbitrary-path write command or dialog save permission; its dialog
permissions cover only opening files/folders and showing messages or confirmations.

QPX names live in `src/lib/qpx.ts`: the views the viewer reads and `COL`, every view column it
reads, checked against the QPX schemas. Queries select only the columns they need and adapt to
each dataset's schema: `anchor_protein` vs `protein_accessions`, a struct- or list-typed
`pg_accessions`, and LFQ or TMT `intensities`. If the `sequence` column contains modification
names, the plain sequence is derived from the peptidoform instead.

<details>
<summary>Project layout</summary>

```
qpxui/
├── src/
│   ├── App.vue                    # shell: top bar, sidebar, main panel
│   ├── store.ts                   # app state (datasets, page, report tab, search)
│   ├── lib/
│   │   ├── api.ts                 # Tauri IPC wrappers + types
│   │   ├── duck.ts                # DuckDB init, view registration, queries
│   │   ├── errors.ts              # cancellation and engine error texts
│   │   ├── qpx.ts                 # QPX view and column names (COL), footer key, sample facets
│   │   ├── queries/               # SQL per tab or view: overview, qc, ptms, samples, proteins,
│   │   │                          #   peptides, quant, profile, header, summary, sqlConsole,
│   │   │                          #   spectrum, table
│   │   ├── report.ts              # shared SQL helpers, dataset tables, query cache, search
│   │   ├── registry.ts            # portal registry search and sort
│   │   ├── charts.ts              # ECharts setup and option builders, CSV export
│   │   ├── colours.ts             # OKLab shades, ramps and mixes; sequential and diverging scales
│   │   ├── sampleColours.ts       # sample colours by biological group (UBERON, UniProt, disease)
│   │   ├── quantCharts.ts         # Quantification tab charts: correlation, PCA, profile
│   │   ├── peptidoformCharts.ts   # peptidoform profile charts: samples, distribution, RT
│   │   ├── spectrumChart.ts       # annotated spectrum chart (PRIDE USI viewer style)
│   │   ├── quant.ts               # normalisation, correlation, clustering, PCA, grouping
│   │   ├── fragments.ts           # peptidoform parsing, fragment ions, annotation
│   │   ├── spectrumSummary.ts     # spectrum statistics, sequence coverage, ion table
│   │   ├── useMirror.ts           # the spectrum viewer's mirror: other PSM, prediction, USI
│   │   ├── mirror.ts              # Koina predictions, PROXI/USI, spectrum similarity
│   │   ├── downloads.ts           # free-space check, confirm, cancel
│   │   ├── progress.ts            # download progress from backend events
│   │   ├── export.ts              # export encoding + Rust save command (PNG / CSV / BibTeX)
│   │   ├── format.ts              # number, size and cell formatting
│   │   ├── ptm.ts, settings.ts    # PTM parsing/colours, theme + settings
│   │   ├── useSlots.ts            # sequential chart loading with progress
│   │   ├── useLargeGate.ts        # hold whole-column reads on large streamed files
│   │   └── *.test.ts              # Vitest unit tests
│   └── components/
│       ├── ReportView.vue         # report header, counts, citation, tabs
│       ├── report/                # Overview, QC, PTMs, Samples (+ SampleFacet), Proteins, Quant
│       │                          #   (+ protein profile), Peptides (+ peptidoform profile), LargeGate
│       ├── SpectrumViewer.vue     # annotated spectrum + mirror
│       ├── spectrum/              # CoverageMap, FragmentIonTable, MirrorControls
│       ├── table/                 # ResultCell (Browse and Query cells), CellMenu, HScroll
│       ├── portal/                # PortalPicker's registry list and dataset heading
│       ├── ChartCard.vue, ReportTable.vue
│       ├── PeptideSeq.vue, PTMSegs.vue, PTMBadges.vue, PTMTip.vue  # PTM-coloured cells
│       ├── SourcePicker.vue, PortalPicker.vue, S3Picker.vue, DownloadProgress.vue
│       ├── DiscoverView.vue, PeptideSearchPane.vue, RecommendedDatasets.vue
│       ├── PortalViews.vue        # a portal dataset's views: stream or download (every portal list)
│       ├── DatasetExplorer.vue    # sidebar: search, datasets, views
│       ├── ViewPanel.vue, DataTable.vue, SummaryView.vue, QueryView.vue
│       └── SettingsPanel.vue, StructuredView.vue
├── src-tauri/
│   ├── tauri.conf.json, Cargo.toml, capabilities/
│   ├── examples/stream_proxy.rs   # manual check of the stream proxy against S3 or the portal
│   └── src/{lib.rs, qpx/, commands/}
├── scripts/                       # fetch-duckdb-extensions.mjs, fetch-sample.mjs, make-icon.py
└── .github/workflows/             # ci.yml (PR checks), release.yml (installers)
```
</details>

---

## Releasing

1. Bump the version in `package.json`, `src-tauri/tauri.conf.json` and `src-tauri/Cargo.toml`.
2. Merge to `main`, then push a tag:

   ```bash
   git tag v0.2.0 && git push origin v0.2.0
   ```

3. The **Release** workflow builds:
   - macOS arm64 and x64 (both on Apple Silicon runners; x64 is cross-compiled);
   - Windows x64;
   - Linux x64.

   It attaches the installers to a **draft** release. Review the draft and publish it.

App icons are generated from `src-tauri/icons/icon-source.png` (drawn by `scripts/make-icon.py`)
with `npx tauri icon src-tauri/icons/icon-source.png`.

---

## Limitations

- AnnData/HDF5 matrices (`.ae.h5ad`, `.de.h5ad`, `.h5mu`) are listed but not yet rendered; the
  viewer focuses on the Parquet views.
- Local files are range-read through the same proxy as remote ones, but every query runs in
  DuckDB-wasm inside the webview: queries that read whole large columns (e.g. spectra in `mz`)
  may hit its memory limits.
- The builds are not code-signed yet (see the macOS note under [Install](#install)).

## License

[Apache-2.0](LICENSE)
