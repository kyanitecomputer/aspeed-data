//! Serde types for the chip definition YAML format (`data/chips/<chip>.yaml`).

use serde::Deserialize;

fn default_arch() -> String {
    "cortex-m".to_owned()
}

/// Top-level chip definition.
///
/// Fields `family`, `core`, `target`, and `num_irqs` are used for AST2700
/// multi-target scaffolding (Task 20) and are kept here for future use.
///
/// # Example
/// ```yaml
/// name: AST2600
/// family: ast26xx
/// core: cortex-m3
/// target: thumbv7m-none-eabi
/// nvic_priority_bits: 3
/// num_irqs: 240
/// peripherals:
///   - name: SSP
///     block: ssp_v1::SSP
///     address: 0x7e6e2a00
/// interrupts:
///   - { name: SDRAM, number: 0 }
/// ```
#[derive(Debug, Deserialize)]
#[allow(dead_code)] // family/core/target/num_irqs used in Task 20 (AST2700)
pub struct ChipDef {
    pub name: String,
    pub family: String,
    pub core: String,
    pub target: String,
    /// CPU architecture string; selects the code-generator backend.
    /// Valid values: `"cortex-m"` (default), `"riscv"`.
    #[serde(default = "default_arch")]
    pub arch: String,
    /// NVIC priority bits.  Absent for RISC-V chips.
    #[serde(default)]
    pub nvic_priority_bits: Option<u8>,
    pub num_irqs: u32,
    #[serde(default)]
    pub peripherals: Vec<PeripheralDef>,
    #[serde(default)]
    pub interrupts: Vec<InterruptDef>,
}

/// A peripheral instance on the chip.
#[derive(Debug, Deserialize)]
pub struct PeripheralDef {
    /// Identifier used in the generated PAC (e.g., `SSP`, `IPC`, `UART11`).
    pub name: String,
    /// Optional doc string.
    #[serde(default)]
    pub description: Option<String>,
    /// Register block reference in the form `"<file_stem>::<BlockName>"`.
    ///
    /// The file stem identifies the register YAML file under `data/registers/`
    /// (e.g., `"ssp_v1"` → `data/registers/ssp_v1.yaml`).
    /// The block name is the chiptool block key within that file (e.g., `"SSP"`).
    #[serde(default)]
    pub block: Option<String>,
    /// Base address in the CPU's virtual address space.
    /// Hex literals (`0x...`) are supported by the YAML 1.1 parser.
    pub address: u64,
}

/// An interrupt line in the NVIC.
#[derive(Debug, Deserialize)]
pub struct InterruptDef {
    /// Rust identifier used in the interrupt enum (must be a valid identifier).
    pub name: String,
    /// NVIC IRQ number (0-indexed from the first peripheral exception).
    pub number: u32,
    /// Optional description emitted as a doc comment.
    #[serde(default)]
    pub description: Option<String>,
}
