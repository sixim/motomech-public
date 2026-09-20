# motomech

**Connect your service manual to your AI assistant — with source grading, applicability guards, and safety gates.**

---

## The problem

You ask your AI assistant: *"What's the valve clearance for my R1200GS?"*

It gives you a number. Probably wrong. BMW ran two completely different engines under the same name — hexhead (2004–2009) and camhead (2010+). Different specs, different tools, different procedures. A wrong torque value doesn't round-trip: it strips threads or under-clamps a safety-critical joint.

The issue isn't that AI is dumb. It's that AI doesn't know *which bike you have*, and service manuals are PDFs that sit on your shelf.

---

## What motomech does

motomech is a **local service manual intelligence layer** between your service manual and your AI assistant.

You bring your manual — as a PDF, or by photographing a physical book with your phone.
motomech indexes it locally (the text never leaves your machine) and exposes a structured
query interface your AI assistant can call tool-by-tool.

```
Your service manual (PDF or physical book)
    ↓  tools/bookproc.sh  (OCR runs locally)
Page index  ← stays on your machine, gitignored
    +
VEHICLE.yml  ← source-graded specs, applicability rules
    ↓  motomech mcp
MCP server
    ↓  tool calls
Claude / Cursor / any MCP-compatible AI
```

When your AI assistant asks `repair.spec("valve clearance")`, it gets back:

```
intake: 0.15 mm  (cold)
exhaust: 0.30 mm  (cold)
source: B | Haynes 4598 · p1•1
applies_to: 2009 R1200GS hexhead
confidence: verified
```

Not a guess. A page reference you can go check.

---

## Key design principles

| | |
|---|---|
| **Files are the source of truth** | SQLite is a rebuildable index. Your specs live in `VEHICLE.yml`, not a database. |
| **Source grading on every fact** | A = official OEM / B = authoritative third-party / C = community / D = inference. The AI knows which is which. |
| **Applicability fail-closed** | A spec tagged for the wrong generation never reaches your query. The filter blocks, never assumes. |
| **Conflict deduction** | Two sources disagree on a value → both are withheld. You decide, not the AI. |
| **Safety gates are deterministic** | Pre-job checklists and hard blocks are YAML rules, not LLM judgment. |

---

## Four modes, one binary

```bash
motomech              # Desktop app (Tauri)
motomech serve        # Local web server (phone-accessible via Tailscale)
motomech mcp          # MCP server for Claude, Cursor, etc.
motomech <command>    # CLI: spec, search, job-card, diag, symptom, log, …
```

---

## Quick start

### 1. Register your vehicle

```bash
# From a VIN — queries NHTSA, generates VEHICLE.yml skeleton automatically
motomech coldstart --vin YOUR_VIN

# Or copy and edit an example manually
cp -r vehicles/2009-BMW-R1200GS vehicles/YEAR-MAKE-MODEL
```

### 2. Add your service manual (three paths)

**You don't need to do this step before querying specs.** If someone has already contributed
a vehicle package with `specs_verified` entries, `repair.spec` works immediately —
no manual processing required.

The manual index unlocks full-text search across all pages. Add it when you need it,
one section at a time if you prefer.

---

**Path A — You have a PDF**

Drop the file in your vehicle's manual directory and run the pipeline:

```bash
mkdir -p vehicles/YEAR-MAKE-MODEL/10_手册资料/third-party/your-manual/raw
cp your-manual.pdf vehicles/YEAR-MAKE-MODEL/10_手册资料/third-party/your-manual/raw/
tools/bookproc.sh -v YEAR-MAKE-MODEL
```

Requires: `brew install ocrmypdf`

---

**Path B — You have a physical book**

Start the local server, then photograph pages with your phone:

```bash
motomech serve --port 8765
```

Open `http://localhost:8765/capture` on your phone (on the same network, or via
[Tailscale serve](https://tailscale.com/kb/1242/tailscale-serve) for HTTPS).
Tap each page to photograph and upload it. Then run the pipeline:

```bash
tools/bookproc.sh -v YEAR-MAKE-MODEL
```

The pipeline handles both PDFs and photos — it detects what's in `raw/` automatically.

---

The OCR text lands in a local `pages/` directory — gitignored, never committed.
The page index (section map, source metadata) is tracked in git and shareable.

### 3. Connect to your AI assistant

Add to your MCP client config (Claude Desktop, Cursor, etc.):

```json
{
  "mcpServers": {
    "motomech": {
      "command": "/path/to/motomech",
      "args": ["mcp"]
    }
  }
}
```

### 4. Build from source

```bash
cd motomech
npm install
cargo build --release
cp target/release/motomech ../bin/
```

Requires Rust (stable) and Node.js 18+.

---

## MCP tool interface

These are the tools your AI assistant can call:

| Tool | What it returns |
|---|---|
| `repair.spec` | Verified spec value with source tier and page reference |
| `repair.search` | Full-text search across your manual pages |
| `repair.job_card` | Step-by-step job card with pre-checked safety gates |
| `repair.safety_check` | Run safety gate rules for a given job |
| `repair.symptom` | Symptom → diagnostic tree walkthrough |
| `repair.diag` | DTC code explanation and isolation steps |
| `repair.script` | Standard procedure scripts (valve check, oil change, etc.) |
| `repair.parts` | Part numbers and sourcing for your vehicle |
| `repair.log` | Append or query the repair log |
| `repair.crossval` | Cross-validate a spec value against multiple sources |
| `repair.gaps` | List specs still missing a verified source |
| `repair.research` | Add a research note or source reference |

All tools accept a vehicle identifier and apply applicability filtering automatically.

---

## Contributing a vehicle

The most valuable contribution is a vehicle package for a make/model you own.

**Step 1:** Generate the skeleton

```bash
motomech coldstart --vin YOUR_VIN
# or: motomech coldstart --make BMW --model "R1200GS" --year 2009
```

**Step 2:** Fill in `VEHICLE.yml` — see [VEHICLE.md](VEHICLE.md) for the schema.

**Step 3:** Add verified specs. Each entry needs a source tier and page number.
Tier A (official OEM) or B (Haynes/Clymer/etc.) only in `specs_verified`.

**Step 4:** Open a PR. CI validates the package format.

The `vehicles/2009-BMW-R1200GS/` directory is the reference implementation —
fully built out with specs, diagnostic trees, wiring index, and job cards.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the full guide.

---

## Repo layout

```
motomech/          Rust/Tauri engine — 4 modes in one binary
  src-tauri/src/
    coldstart.rs      VIN → VEHICLE.yml generation (NHTSA decoder)
    specref.rs        Spec lookup with source grading
    applicability.rs  Three-level applicability filter
    safety.rs         Deterministic safety gate engine
    mcp.rs            MCP server implementation
    …

knowledge/            Cross-vehicle methodology (shared by all vehicles)
  安全门禁/           Safety gate rule definitions
  排查树/             Diagnostic trees (charging, ignition, fuel, …)
  flows/              Multi-step process flows (long-term storage revival, …)

tools/                Processing and utility scripts
  bookproc.sh         PDF → local page index pipeline
  pdf2pages.sh        PDF page extraction
  vlm.py              Local vision model for image-based diagnosis

vehicles/
  2009-BMW-R1200GS/   Reference: hexhead BMW, fully built out
  2014-HD-XL1200/     Skeleton: Sportster, shows a different architecture

VEHICLE.md            Vehicle package format specification
PLATFORM.md           Architecture decisions and design rationale
CONTRIBUTING.md       How to add a vehicle or contribute to the engine
```

---

## Why local-first?

Service manual content is copyrighted. The manual content lives only on your machine, processed by `tools/bookproc.sh`. The repository contains structure, specs, and metadata — never manual text.

This also means your repair data is yours: no cloud sync, no account, no subscription.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).
