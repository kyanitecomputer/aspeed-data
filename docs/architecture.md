# Architecture

## Crate structure

```
aspeed-data/   Host tool — reads YAML, generates PAC
aspeed-pac/    Generated Peripheral Access Crate (DO NOT hand-edit)
embassy-aspeed/ Async HAL crate built on the PAC
```

### aspeed-data (generator)

`aspeed-data-gen` is a host binary (`cargo run -p aspeed-data`). It:
1. Reads every `data/chips/*.yaml` (chip definition)
2. For each peripheral, loads the corresponding `data/registers/<block>.yaml`
3. Namespaces all IR keys with the file stem (`ssp_v1::SSP`, etc.)
4. Merges all register IRs into one, adds the `Device` (interrupt table + peripherals)
5. Applies `ExpandExtends` + `Sort` transforms
6. Calls `chiptool::generate::render()` to produce Rust
7. Writes each chip to `aspeed-pac/src/chips/<feature>.rs`
8. Writes a feature-gated `aspeed-pac/src/lib.rs`

### aspeed-pac (generated PAC)

Never hand-edit. Regenerate with `just generate-pac` after any YAML change.

Feature flags select the chip:
- `ast2600`     → `src/chips/ast2600.rs`
- `ast2700-ssp` → `src/chips/ast2700_ssp.rs`
- `ast2700-tsp` → `src/chips/ast2700_tsp.rs`

### embassy-aspeed (HAL)

Feature `ast2600-ssp` enables all ARM-specific deps (cortex-m, cortex-m-rt, embassy-executor, etc.). Without any chip feature, only pure-logic modules compile (for host unit tests).

## Boot sequence

1. `cortex_m_rt::pre_init` (`boot::pre_init`):
   - Sets VTOR = 0x400 (vector table offset past 1 KB `.sboot` region)
   - Enables ASPEED custom I/D cache (SCUA48 bit 0)
   - Zeroes `RAM_NC` region (`0x0100_0000`–`0x01FF_FFFF`)
2. `cortex_m_rt` zeroes `.bss` and copies `.data` from flash
3. `main()` calls `embassy_aspeed::init()`:
   - Unlocks SCU (writes `0x1688_A8A8` to SCU000)
   - Starts SysTick time driver (200 MHz → 1 µs tick)
4. Embassy executor runs async tasks

## Memory layout (AST2600 SSP)

```
CM3 virtual address   Region       Purpose
0x0000_0000           SBOOT (1 KB) Secure boot header + VTOR padding
0x0000_0400           FLASH (127 KB) .text, .rodata (XIP from DRAM)
0x0002_0000           RAM          .data, .bss, stack, heap
0x0100_0000           RAM_NC       Non-cached: DMA buffers, IPC shared memory
```

Physical DRAM base is configured by the CA7 in `SCUA04` before releasing the CM3.
CM3 virtual `0x0` ↔ physical DRAM start. See `addr.rs` for translation.

## Adding a new peripheral

1. Write `aspeed-data/data/registers/<periph>_v1.yaml` following the chiptool YAML schema.
2. Add a `peripherals:` entry to `aspeed-data/data/chips/ast2600.yaml`:
   ```yaml
   - name:    PERIPH
     block:   periph_v1::PERIPH
     address: 0x7eXXXXXX
   ```
3. `just generate-pac` — regenerates `aspeed-pac/src/`.
4. Implement `embassy-aspeed/src/periph.rs` using raw pointer access or PAC registers.
5. Add `pub mod periph;` to `embassy-aspeed/src/lib.rs`.
6. Write an interrupt handler (if applicable) with `#[no_mangle] pub unsafe extern "C" fn PERIPH_NAME() { ... }`.

## Adding a new SoC

Example: adding AST2700 HAL (currently scaffold only).

1. Create `aspeed-data/data/registers/ssp_v2.yaml` (done — different from v1).
2. Create `aspeed-data/data/chips/ast2700_ssp.yaml` with `target: thumbv7em-none-eabihf`.
3. Add the new feature to `aspeed-pac/Cargo.toml`:
   ```toml
   ast2700-ssp = []
   ```
4. Add feature to `embassy-aspeed/Cargo.toml` `ast2700-ssp = [...]`.
5. Add chip-feature guard to `embassy-aspeed/src/lib.rs` `#[cfg(feature = "ast2700-ssp")]`.
6. `just generate-pac` — generates `aspeed-pac/src/chips/ast2700_ssp.rs`.
7. Write HAL modules gated on `#[cfg(feature = "ast2700-ssp")]`.

## Peripheral version strategy

Each IP version gets a versioned YAML: `uart_v1.yaml`, `ipc_v1.yaml`, `ipc_v2.yaml`.
The chip YAML references the correct version: `block: ipc_v2::IPC`.
HAL code uses `#[cfg]` feature gates when the register layouts differ between versions.

When two chips share the same peripheral layout, they both reference `_v1` and no extra code is needed. When a chip changes the layout (e.g. AST2700 CACHE_FUNC), a `_v2` YAML captures the difference, and the HAL conditionally compiles the correct init sequence.

## Address translation

The CM3 sees its DRAM window starting at virtual `0x0`. The physical start is written by the CA7 into SCUA04 before releasing the CM3. The `addr` module reads this once on first call and caches the result.

For DMA and IPC shared buffers, allocate in `RAM_NC` (place in `.ram_nc` section) and call `to_phys(ptr)` to get the bus address to pass to the CA7.

## SysTick time driver

Embassy's `Timer::after(...)` is backed by the `time_driver` module. It configures SysTick to fire every 200 cycles (1 µs at 200 MHz HCLK). On each tick it increments a u64 counter and drains the waker queue from `embassy-time-queue-utils`. No hardware alarm register is needed — the queue is drained on every tick.

SysTick fires at 1 MHz (1 million interrupts/second). This is intentional: the counter precision is 1 µs. For applications that do not need sub-millisecond precision, consider using the hardware `TIMER` driver as an alarm source instead, and waking the queue less frequently.

## J-Link / probe-rs debugging

The probe-rs target description is at `probe-rs-targets/AST2600_SSP.yaml`. The `.cargo/config.toml` runner uses it automatically via `just flash`. If probe-rs cannot attach (J-Link JTAG chain issues), use J-Link Commander directly:

```
JLinkExe -device Cortex-M3 -if SWD -speed 1000 -autoconnect 1
```

Then in GDB: `target remote :2331`.
