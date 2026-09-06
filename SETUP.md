# Git Setup & Publishing Guide

## 1. Initialize Git Repository

```bash
cd c:\Users\Ash\dev\ascii-dag

# Initialize git
git init

# Add .gitignore
# (Already exists in the repo)

# Add all files
git add .

# Make first commit
git commit -m "Initial release v0.1.0"

# Add GitHub remote
git remote add origin https://github.com/AshutoshMahala/ascii-dag.git

# Push to GitHub
git branch -M main
git push -u origin main
```

## 2. Set Up GitHub Secrets (for CI/CD)

### For Automatic Publishing on Tag:

1. Go to https://crates.io/settings/tokens
2. Create a new API token
3. Go to https://github.com/AshutoshMahala/ascii-dag/settings/secrets/actions
4. Add new secret:
   - Name: `CARGO_TOKEN`
   - Value: (paste your crates.io token)

## 3. Publishing Workflow

### Manual Publishing

```bash
# Verify everything builds
cargo test --all-features
cargo build --release

# Check what will be packaged
cargo package --list

# Test packaging
cargo publish --dry-run

# Publish to crates.io
cargo publish
```

### Automated Publishing (not configured in this checkout)

There are currently no `.github/workflows` files in this checkout.
Configure and verify a publishing workflow before relying on a tag to
publish anything. The steps below describe the intended trigger, not an
existing automation; substitute the release version being published.

```bash
# Tag the release
git tag v0.11.0
git push origin v0.11.0

# A configured workflow should:
# - Run all tests
# - Create a GitHub release
# - Publish to crates.io (if CARGO_TOKEN is set)
```

## 4. After Publishing

Update badges in README.md (they'll work once published):
- [![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
- [![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)

## GitHub Actions CI

Suggested CI coverage, not an installed workflow in this checkout:

✅ **Test on 3 OS**: Ubuntu, Windows, macOS  
✅ **Test on 2 Rust versions**: Stable, Beta  
✅ **Check formatting**: `cargo fmt`  
✅ **Check lints**: `cargo clippy`  
✅ **Test no-std build**: Verify it works without std  
✅ **Run all examples**: Make sure they work  
✅ **Code coverage**: Track test coverage  

## Quick Commands

```bash
# Run all-feature tests locally (also run the feature matrix below)
cargo test --all-features

# Check formatting
cargo fmt --check

# Check clippy
cargo clippy --all-features -- -D warnings -A clippy::too-many-arguments -A clippy::type-complexity

# Build all examples
cargo build --examples

# Check a no-std/no-alloc build (an axis is required)
cargo check --no-default-features --features arena,layout-vertical
```

## Troubleshooting

### "no VCS found" error
```bash
# Initialize git first
git init
git add .
git commit -m "Initial commit"
```

### CI failing on formatting
```bash
# Fix formatting
cargo fmt
git add .
git commit -m "Fix formatting"
```

### CI failing on clippy
```bash
# See clippy warnings
cargo clippy --all-features

# Auto-fix what can be fixed
cargo clippy --fix --allow-dirty
```

## Feature power set (layout axes, 0.11+)

Release validation must run every supported axis combination — the axis
features gate `Direction` variants and whole layout profiles, so each
must build and test independently (plus the standing caution:
`--all-features` unions `arena-idx-u8`, which gates off >255-node arena
tests — always run `--features arena` separately):

```bash
cargo test --features arena                                             # both axes (default)
cargo test --no-default-features --features std,generic,layout-vertical,arena
cargo test --no-default-features --features std,generic,layout-horizontal,arena
cargo check --no-default-features --features arena,layout-vertical      # no-std corners
cargo check --no-default-features --features arena,layout-horizontal
cargo check --no-default-features --features alloc,layout-vertical
cargo test --all-features
```

These are useful smoke checks, not the full feature matrix. The following
shell loop covers the supported library feature sets after accounting
for implication (`std` implies `alloc`; `generic` implies `std`). It
selects one of the three nonempty axis sets, one runtime capability set,
one arena/index choice, and each value of `ports` and `serde` (240 checks).
Select at most one index-width feature. The internal example marker
`embedded_no_std` is not a library capability; build embedded crates
separately with their own target/toolchain.

```bash
for phase6_axes in layout-vertical layout-horizontal layout-vertical,layout-horizontal; do
  for phase6_runtime in "" alloc std generic; do
    for phase6_arena in "" arena arena-idx-u8 arena-idx-u16 arena-idx-u32; do
      for phase6_ports in "" ports; do
        for phase6_serde in "" serde; do
          phase6_features="$phase6_axes"
          for phase6_extra in "$phase6_runtime" "$phase6_arena" "$phase6_ports" "$phase6_serde"; do
            if [ -n "$phase6_extra" ]; then
              phase6_features="$phase6_features,$phase6_extra"
            fi
          done
          cargo check --lib --no-default-features --features "$phase6_features" || exit 1
        done
      done
    done
  done
done
```

Wire this matrix into CI before counting it as a CI gate. An unrestricted
`cargo hack --feature-powerset` also tries configurations with no axis,
which intentionally fail; it needs equivalent supported-set filtering.
Keep an expected-failure check for the no-axis configuration separate
from the successful-build matrix.
