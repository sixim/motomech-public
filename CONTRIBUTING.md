# Contributing to repairbench

There are two ways to contribute:

1. **Add a vehicle package** — the most immediately useful contribution
2. **Improve the engine** — Rust/Tauri code, tools, knowledge structure

---

## Adding a vehicle package

A vehicle package is a directory under `vehicles/<year-make-model>/` with a `VEHICLE.yml`
as the applicability source of truth, plus whatever specs, diagnostic trees, and wiring
information you can verify from your own manual.

### What to contribute

- `VEHICLE.yml` — identity, applicability rules, verified specs (A/B source only)
- `知识/排查树/` — diagnostic trees for common systems (charging, fuel, ignition, etc.)
- `知识/安全门禁/` — vehicle-specific hard rules and pre-job gates
- Section indexes for your manual (page-to-section mapping in `10_手册资料/`)
- Wiring notes in `20_电路系统/`

### What NOT to commit

**Never commit manual text content.** This means:
- No `pages/*.md` files (OCR text from your manual)
- No scanned images or PDFs
- No raw OCR output

The `**/pages/*.md` pattern is in `.gitignore`. The processing pipeline (`tools/bookproc.sh`)
extracts text locally so it stays on your machine.

### Steps

1. Fork the repo and clone locally.

2. Generate your vehicle skeleton:
   ```bash
   repairbench coldstart --vin YOUR_VIN
   # or
   repairbench coldstart --make MAKE --model MODEL --year YEAR
   ```

3. Fill in `VEHICLE.yml`. Required fields:
   - `id`, `make`, `model`, `year`
   - `engine_code` (if `applicability_excludes` is non-empty)
   - `applicability_excludes` — what must NOT match for a resource to apply to your vehicle
   - Set `vin: ~` before opening a PR (remove your actual VIN)

4. Add `specs_verified` entries, each with:
   - A source tier prefix: `A|` (official OEM) or `B|` (Haynes/Clymer/authoritative third-party)
   - A `page:` field pointing to the manual page
   - Do NOT add C or D graded entries to `specs_verified` — use `specs_pending` instead

5. Run validation locally:
   ```bash
   repairbench safety --selftest     # engine self-check (35 gate regression tests)
   repairbench vehicles              # list and validate all vehicle packages
   ```

6. Open a PR. CI runs the same validation.

### Source tier guide

| Tier | Use for | Example |
|---|---|---|
| `A|` | Official OEM service manual | BMW R1200GS Official Workshop Manual |
| `B|` | Authoritative third-party | Haynes 4598, Clymer M502 |
| `C|` | Community consensus | ADVrider wiki, multiple consistent forum posts |
| `D|` | Single source / inference | One forum post, your own reasoning |

Only A and B tiers appear on job cards. C and D go in `specs_pending` with a note.

### Conflict rule

If two A/B-grade sources give different values for the same spec, mark both as a conflict
in `specs_pending` and remove from `specs_verified`. Do not pick one — the system withholds
conflicting values rather than guessing.

---

## Contributing to the engine

### Architecture overview

The engine is a Rust binary with four entrypoints:

```
main.rs            CLI dispatch and Tauri setup
mcp.rs             MCP server (stdio transport, JSON-RPC)
api.rs             HTTP API for the serve mode
cli.rs             CLI command definitions (clap)
```

Core subsystems:

```
specref.rs         Spec lookup — reads VEHICLE.yml, applies source grading
applicability.rs   Three-level filter: make → model (whole-word) → year/generation
safety.rs          Deterministic safety gate engine — reads YAML rule files
coldstart.rs       VIN → VEHICLE.yml generation via NHTSA VIN decoder
db.rs              SQLite schema — rebuildable index, not source of truth
index.rs           Page index builder — reads pages/ text, writes to DB
crossval.rs        Cross-validation: checks multi-source consistency
gaps.rs            Gap analysis: specs referenced but not yet verified
```

### Key invariants (do not break)

1. **`files are truth, db is index`** — any DB query result must be derivable from files alone.
   The DB may be deleted and rebuilt at any time with `repairbench index --rebuild`.

2. **`applicability fail-closed`** — when in doubt, block. A spec that cannot be confirmed
   to apply to the active vehicle must not be returned.

3. **`safety gates are not LLM-negotiable`** — `safety.rs` rules are evaluated deterministically
   before any job card is generated. There is no "override" path.

4. **`C/D graded data never on a job card`** — enforced by a `CHECK` constraint in the DB schema
   and by the job card generator. Do not add a bypass.

### Development setup

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Install Node (for Tauri frontend)
# Use nvm, fnm, or your package manager — Node 18+ required

# Build
cd repairbench
npm install
cargo build

# Run in MCP mode (connect to Claude Desktop for testing)
cargo run -- mcp

# Run CLI
cargo run -- spec "valve clearance"
cargo run -- safety --selftest
```

### Adding a new MCP tool

1. Add the tool definition in `mcp.rs` — the `tools/list` response and `tools/call` dispatch.
2. Implement the handler, usually by calling into an existing subsystem.
3. Update the tool list in `README.md`.
4. Add a test case to `safety --selftest` if the tool touches safety-critical logic.

### PR checklist

- [ ] `cargo clippy` passes (no warnings)
- [ ] `cargo test` passes (run existing tests; add new ones for new logic)
- [ ] `repairbench safety --selftest` passes (35 regression cases)
- [ ] No `unwrap()` on user-facing paths — propagate errors with `?` or return a structured error
- [ ] New MCP tools documented in README.md tool table

---

## Questions and discussion

Open an issue. Tag it `vehicle-package` if it's about adding a new make/model,
`engine` if it's about the Rust code, or `spec-dispute` if you found a spec
that disagrees with your manual.
