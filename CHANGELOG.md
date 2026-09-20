# Changelog

All notable changes to motomech will be documented here.

Format: [Semantic Versioning](https://semver.org/) — `MAJOR.MINOR.PATCH`
- **MAJOR**: breaking changes to vehicle package format or MCP tool interface
- **MINOR**: new features, new MCP tools, new vehicle packages
- **PATCH**: bug fixes, spec corrections, documentation

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

### [0.2.0] — planned
- Web-based setup flow: VIN input + PDF drag-and-drop (no CLI required)
- Pre-built binaries for macOS (Apple Silicon + Intel), Windows, Linux
- Additional vehicle packages

### [0.3.0] — planned
- In-app vehicle package browser
- Community package validation CI

---

[0.1.0]: https://github.com/sixim/motomech-public/releases/tag/v0.1.0
