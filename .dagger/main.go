// Dagger CI module for aspeed-data.
//
// Usage (from the aspeed-data repo root):
//
//	dagger call ci                          # check + gcc-check + test
//	dagger call check                       # compile all crates
//	dagger call gcc-check                   # syntax-check C headers
//	dagger call test                        # host-side unit tests
//	dagger call generate --out ./           # regenerate PAC from YAML
package main

import (
	"context"
	"fmt"
	"strings"

	"dagger/aspeed-data/internal/dagger"
)

const (
	rustChannel = "nightly-2026-04-01"

	// Embedded targets required for PAC checks.
	targetARM   = "thumbv7m-none-eabi"
	targetARMHF = "thumbv7em-none-eabihf"
	targetRISCV = "riscv32imc-unknown-none-elf"
)

// pacTargets lists every (feature, target) combination that must compile.
var pacTargets = []struct{ feature, target string }{
	{"ast2600", targetARM},
	{"ast1060", targetARMHF},
	{"ast2700-ssp", targetARMHF},
	{"ast2700-tsp", targetARMHF},
	{"ast2700-bootmcu", targetRISCV},
}

type AspeedData struct{}

// ── Container builders ────────────────────────────────────────────────────────

// hostContainer returns a Rust nightly container suitable for running the
// host-side generator binary (aspeed-data-gen). No embedded targets needed.
func (m *AspeedData) hostContainer(src *dagger.Directory, chiptool *dagger.Directory) *dagger.Container {
	cargoCache := dag.CacheVolume("cargo-registry")
	buildCache := dag.CacheVolume("cargo-build-aspeed-data-host")

	return dag.Container().
		From("rust:1-slim").
		WithExec([]string{
			"rustup", "toolchain", "install", rustChannel,
			"--profile", "minimal",
			"--component", "rustfmt,clippy",
			"--no-self-update",
		}).
		WithExec([]string{"rustup", "default", rustChannel}).
		WithMountedCache("/usr/local/cargo/registry", cargoCache).
		WithMountedCache("/build/target", buildCache).
		// Mount chiptool at the path the Cargo.toml expects: ../../chiptool
		WithDirectory("/build/chiptool", chiptool).
		WithDirectory("/build/aspeed-data", src).
		WithWorkdir("/build/aspeed-data")
}

// pacContainer returns a Rust nightly container with all three embedded targets
// installed, used for checking the generated aspeed-pac crate.
func (m *AspeedData) pacContainer(src *dagger.Directory) *dagger.Container {
	cargoCache := dag.CacheVolume("cargo-registry")
	buildCache := dag.CacheVolume("cargo-build-aspeed-data-pac")

	return dag.Container().
		From("rust:1-slim").
		WithExec([]string{
			"rustup", "toolchain", "install", rustChannel,
			"--profile", "minimal",
			"--target", targetARM,
			"--target", targetARMHF,
			"--target", targetRISCV,
			"--no-self-update",
		}).
		WithExec([]string{"rustup", "default", rustChannel}).
		WithMountedCache("/usr/local/cargo/registry", cargoCache).
		WithMountedCache("/build/target", buildCache).
		WithDirectory("/build/aspeed-data", src).
		WithWorkdir("/build/aspeed-data")
}

// gccContainer returns a container with GCC for C header syntax checks.
func (m *AspeedData) gccContainer(src *dagger.Directory) *dagger.Container {
	return dag.Container().
		From("gcc:14").
		WithDirectory("/src", src).
		WithWorkdir("/src")
}

// ── Public functions ──────────────────────────────────────────────────────────

// Generate regenerates the Rust PAC and C headers from YAML data.
// Returns the updated repo directory containing the freshly generated files.
//
//	dagger call generate --chiptool ../chiptool export --path ./
func (m *AspeedData) Generate(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	// Path to the chiptool fork (sibling of this repo). Pass: --chiptool ../chiptool
	chiptool *dagger.Directory,
) (*dagger.Directory, error) {
	out, err := m.hostContainer(src, chiptool).
		WithExec([]string{"cargo", "run", "-p", "aspeed-data-gen"}).
		WithExec([]string{"cargo", "fmt", "--manifest-path", "aspeed-pac/Cargo.toml"}).
		Directory("/build/aspeed-data").
		Sync(ctx)
	return out, err
}

// CheckGen compiles the aspeed-data-gen host binary.
// Pass the chiptool sibling dir: --chiptool ../chiptool
func (m *AspeedData) CheckGen(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	chiptool *dagger.Directory,
) error {
	_, err := m.hostContainer(src, chiptool).
		WithExec([]string{"cargo", "check", "-p", "aspeed-data-gen"}).
		Sync(ctx)
	return err
}

// CheckPac compiles the generated aspeed-pac for all five chip targets.
func (m *AspeedData) CheckPac(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
) error {
	ctr := m.pacContainer(src)
	for _, t := range pacTargets {
		_, err := ctr.
			WithExec([]string{
				"cargo", "check",
				"--manifest-path", "aspeed-pac/Cargo.toml",
				"--features", t.feature,
				"--target", t.target,
			}).
			Sync(ctx)
		if err != nil {
			return fmt.Errorf("check aspeed-pac feature=%s target=%s: %w", t.feature, t.target, err)
		}
	}
	return nil
}

// Check compiles all crates: the generator binary and all PAC chip targets.
// Pass the chiptool sibling dir: --chiptool ../chiptool
func (m *AspeedData) Check(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	chiptool *dagger.Directory,
) error {
	if err := m.CheckGen(ctx, src, chiptool); err != nil {
		return fmt.Errorf("check-gen: %w", err)
	}
	return m.CheckPac(ctx, src)
}

// GccCheck runs gcc -fsyntax-only on all generated C headers.
func (m *AspeedData) GccCheck(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
) error {
	headers := []string{
		"aspeed-c-pac/include/ast2400.h",
		"aspeed-c-pac/include/ast2500.h",
	}
	ctr := m.gccContainer(src)
	for _, h := range headers {
		out, err := ctr.
			WithExec([]string{"gcc", "-fsyntax-only", "-std=c11", h}).
			Stderr(ctx)
		if err != nil {
			return fmt.Errorf("gcc check %s: %w\n%s", h, err, out)
		}
	}
	return nil
}

// Test runs host-side unit tests for the generator binary.
// Pass the chiptool sibling dir: --chiptool ../chiptool
func (m *AspeedData) Test(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	chiptool *dagger.Directory,
) error {
	_, err := m.hostContainer(src, chiptool).
		WithExec([]string{"cargo", "test", "-p", "aspeed-data-gen"}).
		Sync(ctx)
	return err
}

// Ci runs the full pipeline: Check + GccCheck + Test.
// Pass the chiptool sibling dir: --chiptool ../chiptool
func (m *AspeedData) Ci(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	chiptool *dagger.Directory,
) (string, error) {
	steps := []string{}

	if err := m.Check(ctx, src, chiptool); err != nil {
		return "", fmt.Errorf("check: %w", err)
	}
	steps = append(steps, "check: ok")

	if err := m.GccCheck(ctx, src); err != nil {
		return "", fmt.Errorf("gcc-check: %w", err)
	}
	steps = append(steps, "gcc-check: ok")

	if err := m.Test(ctx, src, chiptool); err != nil {
		return "", fmt.Errorf("test: %w", err)
	}
	steps = append(steps, "test: ok")

	return strings.Join(steps, "\n"), nil
}
