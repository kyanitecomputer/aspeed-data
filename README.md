# aspeed-data

Single source of truth for ASPEED SoC register definitions.

All output in this repository is **generated** — never edit `aspeed-pac/`, `aspeed-c-pac/`, or `aspeed-go-pac/` by hand. Re-run the generator after any YAML change.

```
https://github.com/kyanitecomputer/aspeed-data
```

## What this repo contains

```
aspeed-data/
├── data/
│   ├── chips/          chip definition YAMLs (one per SoC / coprocessor core)
│   └── registers/      register block YAMLs (versioned, shared across chips)
├── transforms/         chiptool transform pipeline (documented reference)
├── aspeed-data-gen/    host binary that reads YAML and writes all generated outputs
├── aspeed-pac/         GENERATED Rust PAC — do not edit
├── aspeed-c-pac/       GENERATED C headers — do not edit
└── aspeed-go-pac/      GENERATED Go register structs — do not edit (future)
```

## Chip coverage

| Chip | Core | Arch | Rust PAC | C Header | Go structs |
|------|------|------|:--------:|:--------:|:----------:|
| AST1060 | Cortex-M4F | `thumbv7em-none-eabihf` | ✅ | — | planned |
| AST2400 | ColdFire V1 | `m68k-none-elf` | — | ✅ | planned |
| AST2500 | ColdFire V1 | `m68k-none-elf` | — | ✅ | planned |
| AST2600 SSP | Cortex-M3 | `thumbv7m-none-eabi` | ✅ | — | planned |
| AST2700 SSP | Cortex-M4F | `thumbv7em-none-eabihf` | ✅ | — | planned |
| AST2700 TSP | Cortex-M4F | `thumbv7em-none-eabihf` | ✅ | — | planned |
| AST2700 BootMCU | RV32IMC (ibex) | `riscv32imc-unknown-none-elf` | ✅ | — | planned |

## Dependencies

- [`chiptool`](https://github.com/kyanitecomputer/chiptool) — code generator (fork of `embassy-rs/chiptool`; adds C and Go backends). Lives as a sibling directory at `../chiptool`.

## Build and check

All automation uses [Dagger](https://dagger.io). The `chiptool` repo must be a sibling directory.

```sh
# Check all crates compile (PAC only, no chiptool needed)
dagger call check-pac

# Check the generator binary (needs chiptool)
dagger call check-gen --chiptool ../chiptool

# Syntax-check generated C headers
dagger call gcc-check

# Run host-side unit tests
dagger call test --chiptool ../chiptool

# Full CI pipeline
dagger call ci --chiptool ../chiptool

# Regenerate all outputs from YAML (then commit the diff)
dagger call generate --chiptool ../chiptool export --path ./
```

## Development workflow

After changing any YAML file:

1. `dagger call generate --chiptool ../chiptool export --path ./`
2. `dagger call ci --chiptool ../chiptool`
3. Commit both the YAML change and the regenerated outputs together.

Downstream repos (`aspeed-rs`, `aspeed-mcu-runtime`) depend on the generated outputs via relative path deps. Once pushed to GitHub, those deps will switch from `path = ` to `git = "https://github.com/kyanitecomputer/aspeed-data", rev = "<sha>"`.

## Adding a new peripheral

1. Write `data/registers/<periph>_v1.yaml` following the chiptool YAML schema.
2. Add a `peripherals:` entry to the relevant chip YAML in `data/chips/`.
3. Run `dagger call generate --chiptool ../chiptool export --path ./`.
4. Verify with `dagger call ci --chiptool ../chiptool`.

## Adding a new SoC

1. Create `data/chips/<chip>.yaml` with `arch`, `peripherals`, and `interrupts`.
2. For ColdFire chips: set `arch: coldfire` — output goes to `aspeed-c-pac/include/<chip>.h`.
3. For ARM/RISC-V chips: set `arch: cortex-m` or `arch: riscv` — output goes to `aspeed-pac/src/chips/<chip>.rs`.
4. Add the new chip feature to `aspeed-pac/Cargo.toml`.
5. Regenerate and verify.

See `docs/architecture.md` for the full pipeline description.
