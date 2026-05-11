# aspeed-data — Roadmap

Register definition and code generation progress for ASPEED SoC families.

## Current state

All five Rust chip targets compile clean. Both ColdFire C headers pass `gcc -fsyntax-only`. Significant YAML coverage gaps remain for AST2700 and AST2600.

## Phase A: AST2700 peripheral YAML expansion

| Task | Description | Status |
|------|-------------|--------|
| A-1 | `clock_ast2700_v1.yaml` — SCU/clock/PLL for AST2700 system die at `0x72C02000` | ❌ |
| A-2 | Add UART/GPIO/WDT/Timer instances to `ast2700_ssp.yaml` and `ast2700_tsp.yaml` | ❌ |
| A-3 | `ipc0_v1.yaml` — SSP↔CA35 and SSP↔TSP data-channel IPC bus at `0x72C1C000` | ❌ |
| A-4 | `intc0_v1.yaml` + `intc1_v1.yaml` — two-level cascaded interrupt controller | ❌ |

After Phase A: `ast2700_ssp.yaml` should have ≥ 8 peripheral instances.

## Phase B: AST2600 chip YAML expansion

| Task | Description | Status |
|------|-------------|--------|
| B-1 | Add UART1–5, HACE, WDT1–3, I2C0–15, I3C0–7 instances to `ast2600.yaml` | ❌ |

## Phase C: New peripheral type YAMLs

| Task | YAML file | IP | Used by | Status |
|------|-----------|----|---------|----|
| C-1 | `pwm_v1.yaml` | 16-ch PWM + tachometer at `0x7E786000` | AST2600 | ❌ |
| C-2 | `adc_v1.yaml` | 8-ch 10-bit ADC at `0x7E6E9000` | AST2600 | ❌ |
| C-3 | `espi_v1.yaml` | eSPI controller stub at `0x7E6EE000` | AST2600 | ❌ |
| C-4 | `lpc_v1.yaml` | LPC controller stub at `0x7E789000` | AST2600 | ❌ |

## Phase D: Go struct generation

The `chiptool` Go backend (`generate::go`) is implemented. Enabling it requires:

1. Verify Go backend output compiles against `aspeed-go`'s `reg` package.
2. Add a Go routing path in `aspeed-data-gen/src/main.rs` for `arch: go` (new arch value).
3. Add chip YAML entries with `arch: go` or route existing chips to Go output.
4. Populate `aspeed-go-pac/` with generated structs.
5. Add `dagger call generate` Go output step and `go build` verification.

Blocked on: verifying chiptool Go backend compiles for ASPEED register layouts (Task 14 in migration plan).

## Phase E: Publishing

Once YAML coverage is stable and no breaking changes are expected:

- Set `publish = true` in `aspeed-pac/Cargo.toml`.
- Add `CHANGELOG.md`.
- Publish to crates.io with `cargo publish --dry-run`.

## YAML coverage gaps summary

### Register blocks with no HAL driver yet
`hace_v1`, `sgpio_v1`, `uartdma_v1`, `spipf_v1`, `i3c_v1`, `i3cglobal_v1`

### Missing entirely
`clock_ast2700_v1`, `intc0_v1`, `intc1_v1`, `ipc0_v1`, `pwm_v1`, `adc_v1`, `espi_v1`, `lpc_v1`

## Post-push dependency migration

Once pushed to `github.com/kyanitecomputer/aspeed-data`, downstream repos switch from:
```toml
aspeed-pac = { path = "../../aspeed-data/aspeed-pac" }
```
to:
```toml
aspeed-pac = { git = "https://github.com/kyanitecomputer/aspeed-data", rev = "<sha>" }
```
Similarly, the `chiptool` dependency in `aspeed-data-gen/Cargo.toml` switches from `path =` to `git =`.
