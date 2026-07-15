// Dagger CI module for aspeed-data.
//
// All containers use StageX images for reproducible, minimal builds. StageX
// ships a pinned, rustup-less Rust toolchain, so the generator builds on the
// host target directly and the embedded PAC targets are checked with
// -Zbuild-std=core (rust-src is bundled; RUSTC_BOOTSTRAP unlocks it on stable).
//
// Usage (from the aspeed-data repo root):
//
//	dagger call ci --chiptool ../chiptool --aspeed-go ../aspeed-go
//	dagger call check --chiptool ../chiptool
//	dagger call gcc-check
//	dagger call go-check --aspeed-go ../aspeed-go
//	dagger call test --chiptool ../chiptool
//	dagger call generate --chiptool ../chiptool export --path ./
package main

import (
	"context"
	"fmt"
	"strings"

	"dagger/aspeed-data/internal/dagger"
)

const (
	// StageX container images for reproducible builds.
	stagexRust = "stagex/pallet-rust:sx2026.06.0"
	stagexGo   = "stagex/pallet-go:sx2026.06.0"
	stagexGcc  = "stagex/pallet-gcc:sx2026.06.0"

	targetARM   = "thumbv7m-none-eabi"
	targetARMHF = "thumbv7em-none-eabihf"
	targetRISCV = "riscv32imc-unknown-none-elf"
)

var pacTargets = []struct{ feature, target string }{
	{"ast1030", targetARMHF},
	{"ast2600", targetARM},
	{"ast1060", targetARMHF},
	{"ast2700-ssp", targetARMHF},
	{"ast2700-tsp", targetARMHF},
	{"ast2700-bootmcu", targetRISCV},
}

type AspeedData struct{}

// ── Container builders ────────────────────────────────────────────────────────

// hostContainer returns a StageX Rust container for the host-side generator.
func (m *AspeedData) hostContainer(src *dagger.Directory, chiptool *dagger.Directory) *dagger.Container {
	cargoCache := dag.CacheVolume("cargo-registry")
	buildCache := dag.CacheVolume("cargo-build-aspeed-data-host")

	return dag.Container().
		From(stagexRust).
		WithEnvVariable("CARGO_HOME", "/usr/local/cargo").
		WithMountedCache("/usr/local/cargo/registry", cargoCache).
		WithMountedCache("/build/target", buildCache).
		WithDirectory("/build/chiptool", chiptool).
		WithDirectory("/build/aspeed-data", src).
		WithWorkdir("/build/aspeed-data")
}

// pacContainer returns a StageX Rust container for checking the generated
// aspeed-pac crate. StageX has no rustup and no prebuilt bare-metal std, so
// core is compiled per target via -Zbuild-std (see CheckPac); RUSTC_BOOTSTRAP
// unlocks that unstable flag on the pinned stable toolchain.
func (m *AspeedData) pacContainer(src *dagger.Directory) *dagger.Container {
	cargoCache := dag.CacheVolume("cargo-registry")
	buildCache := dag.CacheVolume("cargo-build-aspeed-data-pac")

	return dag.Container().
		From(stagexRust).
		WithEnvVariable("CARGO_HOME", "/usr/local/cargo").
		WithEnvVariable("RUSTC_BOOTSTRAP", "1").
		WithMountedCache("/usr/local/cargo/registry", cargoCache).
		WithMountedCache("/build/target", buildCache).
		WithDirectory("/build/aspeed-data", src).
		WithWorkdir("/build/aspeed-data")
}

// gccContainer returns a StageX GCC container for C header syntax checks.
func (m *AspeedData) gccContainer(src *dagger.Directory) *dagger.Container {
	return dag.Container().
		From(stagexGcc).
		WithDirectory("/src", src).
		WithWorkdir("/src")
}

// ── Public functions ──────────────────────────────────────────────────────────

// Generate regenerates the Rust PAC and C headers from YAML data.
func (m *AspeedData) Generate(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
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

// CheckPac compiles the generated aspeed-pac for all chip targets.
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
				"-Z", "build-std=core",
			}).
			Sync(ctx)
		if err != nil {
			return fmt.Errorf("check aspeed-pac feature=%s target=%s: %w", t.feature, t.target, err)
		}
	}
	return nil
}

// Check compiles all crates: the generator binary and all PAC chip targets.
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

// GoCheck verifies the generated Go PAC compiles using StageX Go.
func (m *AspeedData) GoCheck(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	aspeedGo *dagger.Directory,
) error {
	goCache := dag.CacheVolume("go-mod-cache")
	goBuild := dag.CacheVolume("go-build-cache-aspeed-data")

	_, err := dag.Container().
		From(stagexGo).
		WithMountedCache("/go/pkg/mod", goCache).
		WithMountedCache("/root/.cache/go-build", goBuild).
		WithDirectory("/build/aspeed-data", src).
		WithDirectory("/build/aspeed-go", aspeedGo).
		WithWorkdir("/build/aspeed-data/aspeed-go-pac").
		WithNewFile("/build/aspeed-data/aspeed-go-pac/go.work",
			"go 1.24\nuse .\nuse /build/aspeed-go\n").
		WithExec([]string{"go", "build", "./..."}).
		Sync(ctx)
	return err
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

// Ci runs the full pipeline: Check + GccCheck + GoCheck + Test.
func (m *AspeedData) Ci(
	ctx context.Context,
	// +defaultPath="."
	src *dagger.Directory,
	chiptool *dagger.Directory,
	aspeedGo *dagger.Directory,
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

	if err := m.GoCheck(ctx, src, aspeedGo); err != nil {
		return "", fmt.Errorf("go-check: %w", err)
	}
	steps = append(steps, "go-check: ok")

	if err := m.Test(ctx, src, chiptool); err != nil {
		return "", fmt.Errorf("test: %w", err)
	}
	steps = append(steps, "test: ok")

	return strings.Join(steps, "\n"), nil
}
