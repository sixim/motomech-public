# Changelog

All notable changes to motomech will be documented here.

Format: [Semantic Versioning](https://semver.org/) — `MAJOR.MINOR.PATCH`
- **MAJOR**: breaking changes to vehicle package format or MCP tool interface
- **MINOR**: new features, new MCP tools, new vehicle packages
- **PATCH**: bug fixes, spec corrections, documentation

---

## [0.2.0] — 2026-10-20

### Added
- **Browser setup wizard** at `/setup` (serve mode): enter a VIN → NHTSA decode → `VEHICLE.yml` is created; drag in your manual PDF → saved locally to the vehicle's `raw/`. No command line needed for onboarding.
- **中文 / English toggle** on the setup wizard; defaults to the browser language and remembers your choice.
- **Gus** — the voice of the Q&A layer: a talkative old-school mechanic. Every number Gus says must exist in the fact layer (matched by unit class: N·m, litres, length, page…); answers with an invented number are blocked.
- **Auto-index on first launch**: desktop app, `serve` and `mcp` build the search index from files when it is missing. A fresh clone now shows the reference specs instead of zeros.
- **macOS desktop app** (Apple Silicon `.dmg`) attached to this release.

### Changed
- Display name is now **motomech AI** (window title, page title, intro). The command stays `motomech`.
- Data-folder environment variable is now `MOTOMECH_ROOT`; the old `REPAIRBENCH_ROOT` is still honoured. The desktop app now respects it too.

### Fixed
- `/v1/coldstart` no longer crashes the server: the blocking NHTSA call runs off the async runtime.

### Removed
- Personal data left in the reference vehicle (bike nickname, local Vlog path) and the documentary title in the intro screen.

---

## [0.1.0] — 2026-09-20

Initial public release.

### Engine
- Four-mode binary: desktop (Tauri) / serve / mcp / CLI
- `motomech coldstart --vin <VIN>` — NHTSA VIN decoder → auto-generates VEHICLE.yml
- Source-graded spec lookup: A (official OEM) / B (authoritative third-party) / C (community) / D (inference)
- Three-level applicability filter: make → model (whole-word) → year/generation
- Conflict deduction: two verified sources disagree → both withheld
- Deterministic safety gate engine (35 regression tests)
- MCP server with 12 tools: `repair.spec`, `repair.search`, `repair.job_card`, `repair.safety_check`, `repair.symptom`, `repair.diag`, `repair.script`, `repair.parts`, `repair.log`, `repair.crossval`, `repair.gaps`, `repair.research`
- Cross-validation and gap analysis
- Diagnostic trees (charging, ignition, fuel system, starter, final drive)
- Phone capture endpoint (`/capture`) for photographing physical manuals

### Vehicle packages
- **2009 BMW R1200GS (hexhead)** — reference implementation
  - 25 verified specs (A/B grade), all with page references
  - Applicability rules: excludes camhead, DOHC, LC, water-cooled variants
  - Diagnostic trees for 5 systems
  - Wiring index, parts list, recall records
- **2014 Harley-Davidson XL1200** — skeleton (structure only, specs pending)

### Tools
- `tools/bookproc.sh` — PDF/image → local OCR page index
- `tools/pdf2pages.sh` — PDF page extraction
- `tools/intake.sh` — phone photo intake pipeline
- `tools/install-launchagent.sh` — macOS background service installer

### Documentation
- `VEHICLE.md` — vehicle package format specification
- `CONTRIBUTING.md` — guide for adding vehicles and contributing to the engine
- `PLATFORM.md` — architecture decisions and design rationale

---

## Roadmap

### [0.3.0] — planned
- First-launch folder picker in the desktop app (no env var needed)
- Run the manual OCR/index step from the setup wizard (no CLI)
- Pre-built binaries for macOS Intel, Windows, Linux
- In-app vehicle package browser
- Community package validation CI

---

[0.2.0]: https://github.com/sixim/motomech-public/releases/tag/v0.2.0
[0.1.0]: https://github.com/sixim/motomech-public/releases/tag/v0.1.0
