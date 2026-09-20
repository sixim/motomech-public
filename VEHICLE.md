# Vehicle Package Format

Each vehicle in `vehicles/<year-make-model>/` must have a `VEHICLE.yml` as its
applicability source of truth. This document defines the schema.

---

## Directory layout

```
vehicles/2009-BMW-R1200GS/
├── VEHICLE.yml                   ← required: applicability source of truth
├── README.md                     ← optional: vehicle-specific notes
├── 00_车辆档案/                   ownership docs, recall records
├── 10_手册资料/                   manual metadata and indexes (not the manual text)
│   └── third-party/<name>/
│       ├── TIER                  single character: A / B / C / D
│       ├── 页码映射.tsv           page-to-section index
│       └── pages/                OCR text — gitignored, stays local
├── 20_电路系统/                   wiring diagrams index
├── 40_维修日志/                   repair log
├── 50_零件/                       parts lists
└── 70_规格数据/                   supplementary spec documents
```

---

## VEHICLE.yml schema

### Identity (required)

```yaml
id: 2009-BMW-R1200GS          # <year>-<make>-<model>, used as directory name
make: BMW
model: R1200GS
year: 2009
```

### Engine and drivetrain (required for applicability)

```yaml
engine_code: hexhead          # generation code; used in applicability_excludes
engine_type: air-oil-cooled
displacement_cc: 1170
final_drive: shaft
```

### Identity fields (optional but recommended)

```yaml
vin: ~                        # set to ~ if not sharing; YOUR_VIN_HERE as placeholder
nickname: ~
ecu: ~
diag_port: ~
abs: ~
electrical: ~
```

### Applicability rules (required)

```yaml
applicability_excludes:
  - camhead         # engine_code that must NOT match
  - DOHC
  - LC              # water-cooled variant
  - "R1200GS 2010-2012"
```

All spec lookups and job card generation filter against these terms.
A resource matching any excluded term is silently dropped.

### Hard rules (vehicle-level overrides)

```yaml
hard_rules:
  - id: no-cold-start
    level: BLOCK
    text: "Do not start without completing pre-start checklist."
  - id: manual-crank-first
    level: GATE
    text: "Manually rotate engine before using starter motor."
```

`BLOCK` — operation refused, user directed to professional service.  
`GATE` — checklist required before proceeding.  
`WARN` — allowed, but consequences stated explicitly.

### Verified specs

Each entry in `specs_verified` must have a source tier and page reference.
Tier A (official) or B (authoritative third-party) required to appear on a job card.

```yaml
specs_verified:
  spark_plug:
    type: "Bosch YR5LDE or NGK DCPR 8 EKC"
    gap_standard_mm: "0.7–0.9"
    gap_service_limit_mm: 1.0
    source: "B｜Haynes 4598 · Servicing specifications · Engine"
    page: 21

  valve_clearance:
    intake_mm: 0.15            # cold engine
    exhaust_mm: 0.30           # cold engine
    condition: COLD
    source: "B｜Haynes 4598 · Servicing specifications · Engine"
    page: 21
```

### Pending specs

Items known to be needed but not yet verified:

```yaml
specs_pending:
  - item: ecu_variant
    note: "hexhead uses BMS-K; camhead uses BMS-K+. Confirm for this VIN."
  - item: abs_variant
    note: "Non-servo ABS from 2007+. Verify bleeding procedure."
```

---

## Source tier reference

| Tier | Meaning | May appear on job card? |
|---|---|---|
| A | Official OEM service manual (with page ref) | ✅ Yes |
| B | Authoritative third-party (Haynes/Clymer, with page ref) | ✅ Yes |
| C | Community consensus (multiple independent sources agree) | ⚠️ Hypothesis only |
| D | Single forum post / inference | ❌ No |

Conflicting values at the same tier: both are withheld (neither used).

---

## Validation rules (enforced by CI)

- `id`, `make`, `model`, `year` must be present.
- `engine_code` must be present if `applicability_excludes` is non-empty.
- `specs_verified` entries must have `source` and `page`.
- Source field must start with `A｜`, `B｜`, `C｜`, or `D｜`.
- `vin` must be `~`, `YOUR_VIN_HERE`, or a valid 17-character VIN pattern.
  Real VINs are accepted locally but stripped by the CI check for contributed packages.
- No content from `pages/` directories may be committed.
