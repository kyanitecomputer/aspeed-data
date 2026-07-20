# Architecture

## Repository layout

```
aspeed-data/
├── data/
│   ├── chips/          chip definition YAMLs (one per SoC / coprocessor core)
│   └── registers/      register block YAMLs (versioned, shared across chips)
├── aspeed-data-gen/    host binary that reads YAML and writes all generated outputs
├── aspeed-pac/         GENERATED Rust PAC — do not edit
├── aspeed-c-pac/       GENERATED C headers — do not edit
└── aspeed-go-pac/      GENERATED Go register structs — do not edit
```

`aspeed-pac` and `aspeed-go-pac` are consumed downstream:

| Consumer | How |
|---|---|
| `aspeed-rs` (Embassy HAL) | `aspeed-pac = { path = "../../aspeed-data/aspeed-pac" }` |
| `tamago` (TamaGo SoC packages) | `aspeed-go-pac` via Go module / `go.work` |

`embassy-aspeed` is **not** part of this repo — it lives in `aspeed-rs`.

---

## Generator (`aspeed-data-gen`)

`aspeed-data-gen` is a host binary. Run it with:

```sh
cargo run -p aspeed-data-gen
```

Or via the Dagger pipeline (recommended — also formats and verifies):

```sh
dagger call generate --chiptool ../chiptool export --path ./
```

The generator:
1. Reads every `data/chips/*.yaml` (chip definition)
2. For each peripheral, loads `data/registers/<block>.yaml`
3. Namespaces all IR keys with the file stem (`ssp_v1::SSP`, etc.)
4. Merges register IRs into a single `Device` (interrupt table + peripherals)
5. Applies `ExpandExtends` + `Sort` chiptool transforms
6. Calls `chiptool::generate::rust::render()` / `::c::render()` / `::go::render()`
7. Writes `aspeed-pac/src/chips/<chip>.rs`, `aspeed-c-pac/include/<chip>.h`,
   and `aspeed-go-pac/<chip>/*.gen.go`
8. Writes a feature-gated `aspeed-pac/src/lib.rs`

Only write files when content changes (`write_if_changed`) to avoid spurious
rebuilds.

---

## Generated PAC (`aspeed-pac`)

Never hand-edit. Regenerate after any YAML change.

Feature flags select the chip. Each flag is mutually exclusive in practice:

| Feature | Target | Output file |
|---|---|---|
| `ast1060` | `thumbv7em-none-eabihf` | `src/chips/ast1060.rs` |
| `ast2600` | `thumbv7m-none-eabi` | `src/chips/ast2600.rs` |
| `ast2700-ssp` | `thumbv7em-none-eabihf` | `src/chips/ast2700_ssp.rs` |
| `ast2700-tsp` | `thumbv7em-none-eabihf` | `src/chips/ast2700_tsp.rs` |
| `ast2700-bootmcu` | `riscv32imc-unknown-none-elf` | `src/chips/ast2700_bootmcu.rs` |

The `defmt` feature adds `#[derive(defmt::Format)]` to enums and structs
in the generated PAC.

---

## Adding a new peripheral

1. Write `data/registers/<periph>_v1.yaml` following the chiptool YAML schema.
2. Add a `peripherals:` entry to the relevant chip YAML in `data/chips/`:
   ```yaml
   peripherals:
     - name:    PERIPH
       block:   periph_v1::PERIPH
       address: 0x14CXXXXX
   ```
3. Run `cargo run -p aspeed-data-gen` (or `dagger call generate ...`).
4. Verify with `cargo check --manifest-path aspeed-pac/Cargo.toml --features <chip> --target <triple>`.

---

## Adding a new SoC

1. Create `data/chips/<chip>.yaml` with `arch`, `peripherals`, and `interrupts`.
   - ARM/RISC-V: `arch: cortex-m` or `arch: riscv` → Rust PAC output.
   - ColdFire: `arch: coldfire` → C header output.
2. Add the new chip feature to `aspeed-pac/Cargo.toml`.
3. Run the generator and verify.

---

## Peripheral version strategy

Each IP version gets a versioned YAML: `uart_v1.yaml`, `ssp_v1.yaml`, `ssp_v2.yaml`.
The chip YAML references the correct version: `block: ssp_v2::SSP`.

When two chips share the same peripheral layout they both reference `_v1` and
no extra code is needed. When a chip changes the layout a new `_v2` YAML
captures the delta.

---

## Boot sequence — AST2600 SSP (Cortex-M3)

1. `cortex_m_rt::pre_init` (`boot::pre_init` in `aspeed-rs`):
   - Sets VTOR = 0x400 (vector table past 1 KB `.sboot` region)
   - Enables ASPEED I/D cache (SCUA48 bit 0)
   - Zeroes `RAM_NC` region (`0x0100_0000`–`0x01FF_FFFF`)
2. `cortex_m_rt` zeroes `.bss` and copies `.data` from flash
3. `main()` calls `embassy_aspeed::init()`:
   - Unlocks SCU (writes `0x1688_A8A8` to SCU000)
   - Starts SysTick time driver (200 MHz → 1 µs tick)
4. Embassy executor runs async tasks

### Memory layout (AST2600 SSP)

```
CM3 virtual address   Region       Purpose
0x0000_0000           SBOOT (1 KB) Secure boot header + VTOR padding
0x0000_0400           FLASH        .text, .rodata (XIP from DRAM)
0x0002_0000           RAM          .data, .bss, stack
0x0100_0000           RAM_NC       Non-cached: DMA buffers, IPC shared memory
```

Physical DRAM base is configured by the CA7 into SCUA04 before releasing the
CM3. CM3 virtual `0x0` ↔ physical DRAM start. See `addr.rs` in `aspeed-rs`.

---

## Boot sequence — AST2700 BootMCU (RV32IMC ibex)

The BootMCU is the Root of Trust processor. It runs from SRAM loaded by the
SoC ROM; DRAM is not available until the BootMCU trains it.

1. SoC ROM copies FMC binary from SPI flash to GSRAM at `0x14B80A00`
2. ROM handles Caliptra bring-up (FMC arrives with Caliptra in `RDY_FOR_RT`)
3. BootMCU FMC (`riscv-rt` + Embassy executor):
   - `.init` stub: `csrwi mie,0` before riscv-rt `_start` (interrupt race fix)
   - `boot_rv::platform_init_rv()`: vectored interrupt table for ibex
   - `wdt_ast2700::init()` + `extrst::init()`: WDT/EXTRST reset masks
   - `sli::init_f()` / `sli::init_r()`: System Link Interface calibration
   - `scu::apply_ibex_default_register_policy()`: SCU security policy
   - `sdrammc::init()`: DDR4/DDR5 PHY training, MRS, BIST, size detect
   - `ca35::init_vendor_runtime_fabric()`: fabric/PCI/UFS gates
   - Load CA35 payload + SSP/TSP firmware from SPI flash XIP
   - `ca35::enable_vendor_mpu_regions()`, `ca35::set_rvbar()`, `ca35::release()`
   - `ssp_tsp::enable_ssp()` / `ssp_tsp::enable_tsp()`
   - Maintenance loop: Caliptra TRNG feeding, watchdog, IPC

### Memory layout (AST2700 BootMCU, pre-DRAM)

```
BootMCU address    Size      Type         Purpose
0x1000_0000        128 KB    ECC SRAM     System-die; DMA buffers
0x12C0_0000        4 KB      SDRAMMC      DRAM controller registers
0x1300_0000        512 KB    DDR PHY      PHY training IMEM/DMEM
0x14B8_0000        2560 B    GSRAM        ASTH header (placed by ROM)
0x14B8_0A00        ~187 KB   GSRAM        FMC binary (.text+.bss+stack)
0x14C0_2000        8 KB      SCU1         IO-die system control
0x14C3_3B00        256 B     UART12       115200 8N1 (ROM-configured)
0x14C3_6000        64 B      Timer        1 MHz, 64-bit, vectored IRQ 7
0x14C6_0000        4 KB      Caliptra     Mailbox command interface
0x14C7_0000        4 KB      Caliptra     IFC boot flow / TRNG / fuse
0x2000_0000        512 MB    SPI XIP      Read-only flash mapping
0x8000_0000        1 GB      SDRAM        Unavailable until DRAM training
```

---

## SysTick time driver (AST2600 SSP / AST1060)

Embassy's `Timer::after(...)` is backed by the `time_driver` module in
`aspeed-rs`. It configures SysTick to fire every 200 cycles (1 µs at 200 MHz
HCLK). On each tick it increments a u64 counter and drains the waker queue.

For the AST2700 BootMCU a separate `time_driver_rv.rs` uses the 64-bit
hardware timer at `0x14C36000` with vectored IRQ 7.
