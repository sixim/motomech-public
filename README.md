# repairbench

**Connect your service manual to your AI assistant — with source grading, applicability guards, and safety gates.**

---

## The problem

You ask your AI assistant: *"What's the valve clearance for my R1200GS?"*

It gives you a number. Probably wrong. BMW ran two completely different engines under the same name — hexhead (2004–2009) and camhead (2010+). Different specs, different tools, different procedures. A wrong torque value doesn't round-trip: it strips threads or under-clamps a safety-critical joint.

The issue isn't that AI is dumb. It's that AI doesn't know *which bike you have*, and service manuals are PDFs that sit on your shelf.

---

## What repairbench does

repairbench is a **local service manual intelligence layer** between your PDF and your AI assistant.

```
Your PDF manual
    ↓  tools/bookproc.sh
Local page index (stays on your machine, gitignored)
    ↓  repairbench mcp
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
repairbench              # Desktop app (Tauri)
repairbench serve        # Local web server (phone-accessible via Tailscale)
repairbench mcp          # MCP server for Claude, Cursor, etc.
repairbench <command>    # CLI: spec, search, job-card, diag, symptom, log, …
```

---

## Quick start

### 1. Register your vehicle

```bash
# From a VIN — queries NHTSA, generates VEHICLE.yml skeleton automatically
repairbench coldstart --vin YOUR_VIN

# Or copy and edit an example manually
cp -r vehicles/2009-BMW-R1200GS vehicles/YEAR-MAKE-MODEL
```

### 2. Process your service manual

```bash
# Drop your PDF into your vehicle's manual directory, then:
tools/bookproc.sh vehicles/YEAR-MAKE-MODEL/10_手册资料/your-manual.pdf
```

The OCR text is extracted to a local `pages/` directory — gitignored, stays on your machine.
The page index (headings, section map, source metadata) is tracked in git.

### 3. Connect to your AI assistant

Add to your MCP client config (Claude Desktop, Cursor, etc.):

```json
{
  "mcpServers": {
    "repairbench": {
      "command": "/path/to/repairbench",
      "args": ["mcp"]
    }
  }
}
```

### 4. Build from source

```bash
cd repairbench
npm install
cargo build --release
cp target/release/repairbench ../bin/
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
repairbench coldstart --vin YOUR_VIN
# or: repairbench coldstart --make BMW --model "R1200GS" --year 2009
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
repairbench/          Rust/Tauri engine — 4 modes in one binary
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
