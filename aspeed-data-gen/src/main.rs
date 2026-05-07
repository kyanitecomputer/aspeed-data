//! ASPEED SoC PAC generator (`aspeed-data-gen`).
//!
//! Reads chip definitions from `data/chips/<chip>.yaml` and register
//! descriptions from `data/registers/<block>.yaml`, constructs a chiptool IR,
//! applies transforms, and renders the generated PAC.
//!
//! # Multi-chip output
//!
//! Each chip YAML is rendered into a separate Rust module file under
//! `aspeed-pac/src/chips/<chip>.rs`.  A feature-gated `lib.rs` re-exports
//! the selected chip's symbols.
//!
//! Chip-to-feature mapping (chip YAML `name` field → Cargo feature):
//! - `AST2600`     → `ast2600`
//! - `AST2700_SSP` → `ast2700-ssp`
//! - `AST2700_TSP` → `ast2700-tsp`
//!
//! # Usage
//!
//! ```sh
//! cargo run -p aspeed-data-gen    # regenerate PAC
//! dagger call generate            # regenerate + rustfmt (recommended)
//! ```

mod chip;

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use chiptool::generate::c::{CArch, CGenerator, COptions};
use chiptool::generate::rust::{CommonModule, DefmtOption, Options, RustArch};
use chiptool::generate::CodeGenerator;
use chiptool::ir::{
    Block, BlockItemInner, Device, Enum, FieldSet, Interrupt, Peripheral, IR,
};
use chiptool::generate::rust::render as rust_render;
use chiptool::transform::expand_extends::ExpandExtends;
use chiptool::transform::sort::Sort;

fn main() -> Result<()> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));

    // The crate lives at <repo>/aspeed-data-gen/; data lives at <repo>/data/.
    let workspace_dir = manifest_dir
        .parent()
        .context("aspeed-data-gen has no parent dir")?;

    let data_dir = workspace_dir.join("data");

    // Rust PAC output directory.
    let pac_src_dir = workspace_dir.join("aspeed-pac").join("src");
    // C PAC output directory (single-file headers, one per ColdFire chip).
    let c_pac_dir = workspace_dir.join("aspeed-c-pac").join("include");

    eprintln!("aspeed-data-gen");
    eprintln!("  data dir   : {}", data_dir.display());
    eprintln!("  rust output: {}", pac_src_dir.display());
    eprintln!("  c output   : {}", c_pac_dir.display());

    std::fs::create_dir_all(&pac_src_dir)?;
    let chips_subdir = pac_src_dir.join("chips");
    std::fs::create_dir_all(&chips_subdir)?;
    std::fs::create_dir_all(&c_pac_dir)?;

    // Process every *.yaml in data/chips/ (sorted, deterministic).
    let chips_dir = data_dir.join("chips");
    let mut entries: Vec<_> = std::fs::read_dir(&chips_dir)
        .with_context(|| format!("reading {}", chips_dir.display()))?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("yaml"))
        .collect();
    entries.sort_by_key(|e| e.path());

    let mut chip_infos: Vec<(String, String)> = Vec::new(); // (chip_name, feature_flag)

    for entry in &entries {
        let path = entry.path();
        eprintln!("  chip: {}", path.display());

        // Peek at the arch field to pick the right backend without double-parsing the
        // full IR (the arch field is in the chip YAML, not the register YAMLs).
        let arch = chip_arch(&path)?;

        if arch == "coldfire" {
            process_chip_c(&path, &data_dir, &c_pac_dir)?;
        } else {
            let (chip_name, feature) = process_chip(&path, &data_dir, &chips_subdir)?;
            chip_infos.push((chip_name, feature));
        }
    }

    // Generate the feature-gated lib.rs for Rust chips only.
    let lib_rs = generate_lib_rs(&chip_infos);
    let lib_path = pac_src_dir.join("lib.rs");
    write_if_changed(&lib_path, &lib_rs)
        .with_context(|| format!("writing {}", lib_path.display()))?;
    eprintln!("    wrote  : {}", lib_path.display());

    eprintln!("  done ({} chip(s))", chip_infos.len());
    Ok(())
}

/// Read just the `arch` field from a chip YAML without loading register files.
fn chip_arch(chip_path: &Path) -> Result<String> {
    let yaml = std::fs::read_to_string(chip_path)
        .with_context(|| format!("reading {}", chip_path.display()))?;
    let chip: chip::ChipDef = serde_yaml::from_str(&yaml)
        .with_context(|| format!("parsing {}", chip_path.display()))?;
    Ok(chip.arch)
}

// ── Per-chip processing ───────────────────────────────────────────────────────

/// Process one chip YAML, write the rendered module to `out_dir/<module>.rs`.
/// Returns `(chip_name, feature_flag)`.
fn process_chip(chip_path: &Path, data_dir: &Path, out_dir: &Path) -> Result<(String, String)> {
    let yaml = std::fs::read_to_string(chip_path)
        .with_context(|| format!("reading {}", chip_path.display()))?;

    let chip: chip::ChipDef = serde_yaml::from_str(&yaml)
        .with_context(|| format!("parsing {}", chip_path.display()))?;

    let mut ir = IR::new();

    // Load and namespace register YAMLs.
    let mut loaded: std::collections::BTreeSet<String> = Default::default();
    for periph in &chip.peripherals {
        let block_ref = match &periph.block {
            Some(b) => b,
            None => continue,
        };
        let file_stem = block_ref
            .split("::")
            .next()
            .with_context(|| format!("block ref '{}' must contain '::'", block_ref))?
            .to_owned();
        if !loaded.insert(file_stem.clone()) {
            continue;
        }
        let reg_path = data_dir
            .join("registers")
            .join(format!("{}.yaml", file_stem));
        eprintln!("    registers: {}", reg_path.display());
        let reg_yaml = std::fs::read_to_string(&reg_path)
            .with_context(|| format!("reading {}", reg_path.display()))?;
        let mut reg_ir: IR = serde_yaml::from_str(&reg_yaml)
            .with_context(|| format!("parsing {}", reg_path.display()))?;
        namespace_ir(&mut reg_ir, &file_stem);
        ir.merge(reg_ir);
    }

    // Build the Device entry.
    let device = Device {
        nvic_priority_bits: chip.nvic_priority_bits,
        peripherals: chip
            .peripherals
            .iter()
            .filter_map(|p| {
                p.block.as_ref().map(|block_ref| Peripheral {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    base_address: p.address,
                    array: None,
                    block: Some(block_ref.clone()),
                    interrupts: BTreeMap::new(),
                })
            })
            .collect(),
        interrupts: chip
            .interrupts
            .iter()
            .map(|i| Interrupt {
                name: i.name.clone(),
                description: i.description.clone(),
                value: i.number,
            })
            .collect(),
    };

    // Use the chip name as the device key.
    ir.devices.insert(chip.name.clone(), device);

    // Apply transforms.
    ExpandExtends {}.run(&mut ir).context("ExpandExtends")?;
    Sort {}.run(&mut ir).context("Sort")?;

    // Select architecture backend from chip YAML.
    let arch = match chip.arch.as_str() {
        "riscv" => RustArch::RiscV,
        _ => RustArch::CortexM,
    };

    // Render to Rust.
    let opts = Options::new()
        .with_common_module(CommonModule::Builtin)
        .with_defmt(DefmtOption::Feature("defmt".to_owned()))
        .with_skip_no_std(false)
        .with_arch(arch);
    let tokens = rust_render(&ir, &opts).context("generate::render")?;
    let raw = tokens.to_string();
    let parsed = syn::parse_file(&raw).context("syn failed to parse generated code")?;
    let formatted = prettyplease::unparse(&parsed);

    let feature = chip_name_to_feature(&chip.name);
    let module_name = feature.replace('-', "_");
    // Strip only `#![no_std]` — it's a crate-level attr that's invalid in
    // module files.  All other inner attrs (e.g. `#![allow(...)]`) are kept
    // because they apply to the module when included via `#[path = "..."]`.
    let module_content: String = formatted
        .lines()
        .filter(|l| l.trim() != "#![no_std]")
        .collect::<Vec<_>>()
        .join("\n");
    let module_content = format!(
        "// Generated by aspeed-data-gen. DO NOT EDIT.\n{}\n",
        module_content.trim_start_matches('\n')
    );

    let out_path = out_dir.join(format!("{}.rs", module_name));
    write_if_changed(&out_path, &module_content)
        .with_context(|| format!("writing {}", out_path.display()))?;
    eprintln!("    wrote  : {}", out_path.display());

    Ok((chip.name, feature))
}

// ── ColdFire C header generation ─────────────────────────────────────────────

/// Process one ColdFire chip YAML, write a merged single-file C header to
/// `out_dir/<chip_name_lower>.h`.
///
/// The output is a CMSIS-style header:
/// - One `typedef struct` per peripheral block (with padding for gaps).
/// - `_Pos` / `_Msk` macros for every fieldset field.
/// - `#define <PERIPH>_BASE` and `#define <PERIPH>` pointer macros.
/// - `#define <NAME>_IRQn` defines for every interrupt.
fn process_chip_c(chip_path: &Path, data_dir: &Path, out_dir: &Path) -> Result<()> {
    let yaml = std::fs::read_to_string(chip_path)
        .with_context(|| format!("reading {}", chip_path.display()))?;
    let chip: chip::ChipDef = serde_yaml::from_str(&yaml)
        .with_context(|| format!("parsing {}", chip_path.display()))?;

    let mut ir = IR::new();

    // Load and namespace register YAMLs (same as process_chip).
    let mut loaded: std::collections::BTreeSet<String> = Default::default();
    for periph in &chip.peripherals {
        let block_ref = match &periph.block {
            Some(b) => b,
            None => continue,
        };
        let file_stem = block_ref
            .split("::")
            .next()
            .with_context(|| format!("block ref '{}' must contain '::'", block_ref))?
            .to_owned();
        if !loaded.insert(file_stem.clone()) {
            continue;
        }
        let reg_path = data_dir
            .join("registers")
            .join(format!("{}.yaml", file_stem));
        eprintln!("    registers: {}", reg_path.display());
        let reg_yaml = std::fs::read_to_string(&reg_path)
            .with_context(|| format!("reading {}", reg_path.display()))?;
        let mut reg_ir: IR = serde_yaml::from_str(&reg_yaml)
            .with_context(|| format!("parsing {}", reg_path.display()))?;
        namespace_ir(&mut reg_ir, &file_stem);
        ir.merge(reg_ir);
    }

    let device = Device {
        nvic_priority_bits: chip.nvic_priority_bits,
        peripherals: chip
            .peripherals
            .iter()
            .filter_map(|p| {
                p.block.as_ref().map(|block_ref| Peripheral {
                    name: p.name.clone(),
                    description: p.description.clone(),
                    base_address: p.address,
                    array: None,
                    block: Some(block_ref.clone()),
                    interrupts: BTreeMap::new(),
                })
            })
            .collect(),
        interrupts: chip
            .interrupts
            .iter()
            .map(|i| Interrupt {
                name: i.name.clone(),
                description: i.description.clone(),
                value: i.number,
            })
            .collect(),
    };

    ir.devices.insert(chip.name.clone(), device);

    ExpandExtends {}.run(&mut ir).context("ExpandExtends")?;
    Sort {}.run(&mut ir).context("Sort")?;

    // Run the C generator (returns one file per block + one device file).
    let opts = COptions {
        include_guard_prefix: "PAC_".to_string(),
        arch: CArch::ColdFire,
    };
    let generator = CGenerator;
    let files = generator
        .generate(&ir, &opts)
        .context("CGenerator::generate")?;

    // Merge all generated files into a single self-contained header.
    let header = merge_c_headers(&chip.name, &files);

    // Verify the merged header compiles with GCC if available.
    // (Non-fatal — just a best-effort check.)
    check_c_syntax(&chip.name, &header);

    let out_name = chip.name.to_lowercase().replace('_', "-");
    let out_path = out_dir.join(format!("{}.h", out_name));
    write_if_changed(&out_path, &header)
        .with_context(|| format!("writing {}", out_path.display()))?;
    eprintln!("    wrote  : {}", out_path.display());

    Ok(())
}

/// Merge per-file CGenerator output into a single self-contained C header.
///
/// Strips individual include guards and `#include <stdint.h>` / `#include "pac/…"`
/// lines from each file, then wraps everything in one chip-level include guard.
fn merge_c_headers(chip_name: &str, files: &[(String, String)]) -> String {
    let guard = format!("ASPEED_{}_H", chip_name.to_uppercase());
    let mut out = String::new();

    out.push_str("/* Auto-generated by aspeed-data-gen. DO NOT EDIT. */\n");
    out.push_str(&format!("#ifndef {guard}\n#define {guard}\n\n"));
    out.push_str("#include <stdint.h>\n\n");

    // Inline all block (pac/*.h) files first — struct typedefs and field macros.
    for (name, content) in files {
        if name.starts_with("pac/") {
            out.push_str(&strip_guards_and_includes(content));
            out.push('\n');
        }
    }

    // Inline the device header — peripheral instance macros and IRQ defines.
    for (name, content) in files {
        if name.starts_with("device_") {
            out.push_str(&strip_guards_and_includes(content));
        }
    }

    out.push_str(&format!("\n#endif /* {guard} */\n"));
    out
}

/// Remove include guard lines, `#include <stdint.h>`, and `#include "pac/…"` lines
/// from a generated C file so it can be safely inlined into a merged header.
fn strip_guards_and_includes(content: &str) -> String {
    content
        .lines()
        .filter(|line| {
            let t = line.trim();
            if t.starts_with("#ifndef ") && t.ends_with("_H") {
                return false;
            }
            if t.starts_with("#define ") {
                let parts: Vec<&str> = t.split_whitespace().collect();
                if parts.len() == 2 && parts[1].ends_with("_H") {
                    return false;
                }
            }
            if t.starts_with("#endif") && t.contains("_H") {
                return false;
            }
            if t == "#include <stdint.h>" {
                return false;
            }
            if t.starts_with("#include \"pac/") {
                return false;
            }
            true
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run `gcc -fsyntax-only -std=c11` on the generated header as a best-effort
/// compile check.  Prints a warning but does not abort if gcc is not found.
fn check_c_syntax(chip_name: &str, content: &str) {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = match Command::new("gcc")
        .args(["-fsyntax-only", "-std=c11", "-x", "c", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(c) => c,
        Err(_) => {
            eprintln!("    [warn] gcc not found; skipping syntax check for {}", chip_name);
            return;
        }
    };

    if let Some(stdin) = child.stdin.as_mut() {
        let _ = stdin.write_all(content.as_bytes());
    }

    match child.wait_with_output() {
        Ok(out) if out.status.success() => {
            eprintln!("    [ok]   gcc -fsyntax-only passed for {}", chip_name);
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            eprintln!("    [FAIL] gcc -fsyntax-only failed for {}:\n{}", chip_name, stderr);
        }
        Err(e) => {
            eprintln!("    [warn] gcc check error for {}: {}", chip_name, e);
        }
    }
}

// ── Feature flag mapping ──────────────────────────────────────────────────────

fn chip_name_to_feature(name: &str) -> String {
    match name {
        "AST2600"     => "ast2600".to_owned(),
        "AST2700_SSP" => "ast2700-ssp".to_owned(),
        "AST2700_TSP" => "ast2700-tsp".to_owned(),
        other         => other.to_lowercase().replace('_', "-"),
    }
}

fn feature_to_module(feature: &str) -> String {
    feature.replace('-', "_")
}

// ── lib.rs generation ─────────────────────────────────────────────────────────

fn generate_lib_rs(chips: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str("// Generated by aspeed-data-gen.  DO NOT EDIT.\n");
    out.push_str("// Re-run `cargo run -p aspeed-data-gen` to regenerate.\n\n");
    out.push_str("#![no_std]\n\n");

    // Include each chip module under its feature flag.
    for (chip_name, feature) in chips {
        let module = feature_to_module(feature);
        out.push_str(&format!("// {}\n", chip_name));
        out.push_str(&format!("#[cfg(feature = \"{}\")]\n", feature));
        out.push_str(&format!("#[path = \"chips/{}.rs\"]\n", module));
        out.push_str(&format!("mod {};\n", module));
        // Re-export everything from the selected chip module.
        out.push_str(&format!("#[cfg(feature = \"{}\")]\n", feature));
        out.push_str(&format!("pub use {}::*;\n\n", module));
    }

    out
}

// ── IR namespacing ────────────────────────────────────────────────────────────

fn namespace_ir(ir: &mut IR, prefix: &str) {
    let pfx = |name: &str| -> String { format!("{}::{}", prefix, name) };

    let old_blocks: BTreeMap<String, Block> = std::mem::take(&mut ir.blocks);
    for (k, v) in old_blocks {
        ir.blocks.insert(pfx(&k), v);
    }
    let old_fieldsets: BTreeMap<String, FieldSet> = std::mem::take(&mut ir.fieldsets);
    for (k, v) in old_fieldsets {
        ir.fieldsets.insert(pfx(&k), v);
    }
    let old_enums: BTreeMap<String, Enum> = std::mem::take(&mut ir.enums);
    for (k, v) in old_enums {
        ir.enums.insert(pfx(&k), v);
    }

    for block in ir.blocks.values_mut() {
        if let Some(ext) = &mut block.extends {
            *ext = pfx(ext);
        }
        for item in &mut block.items {
            match &mut item.inner {
                BlockItemInner::Register(reg) => {
                    if let Some(fs) = &mut reg.fieldset {
                        *fs = pfx(fs);
                    }
                }
                BlockItemInner::Block(b) => {
                    b.block = pfx(&b.block);
                }
            }
        }
    }

    for fieldset in ir.fieldsets.values_mut() {
        if let Some(ext) = &mut fieldset.extends {
            *ext = pfx(ext);
        }
        for field in &mut fieldset.fields {
            if let Some(e) = &mut field.enumm {
                *e = pfx(e);
            }
        }
    }
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn write_if_changed(path: &Path, content: &str) -> Result<()> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == content {
            return Ok(());
        }
    }
    std::fs::write(path, content)?;
    Ok(())
}
