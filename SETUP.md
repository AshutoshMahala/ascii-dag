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

### Manual Publishing (Recommended for v0.1.0)

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

### Automated Publishing (Future Releases)

```bash
# Tag the release
git tag v0.1.0
git push origin v0.1.0

# GitHub Actions will automatically:
# - Run all tests
# - Create a GitHub release
# - Publish to crates.io (if CARGO_TOKEN is set)
```

## 4. After Publishing

Update badges in README.md (they'll work once published):
- [![Crates.io](https://img.shields.io/crates/v/ascii-dag.svg)](https://crates.io/crates/ascii-dag)
- [![Documentation](https://docs.rs/ascii-dag/badge.svg)](https://docs.rs/ascii-dag)

## GitHub Actions CI

The CI will automatically run on every push and PR:

✅ **Test on 3 OS**: Ubuntu, Windows, macOS  
✅ **Test on 2 Rust versions**: Stable, Beta  
✅ **Check formatting**: `cargo fmt`  
✅ **Check lints**: `cargo clippy`  
✅ **Test no-std build**: Verify it works without std  
✅ **Run all examples**: Make sure they work  
✅ **Code coverage**: Track test coverage  

## Quick Commands

```bash
# Run all tests locally (same as CI)
cargo test --all-features

# Check formatting
cargo fmt --check

# Check clippy (with same settings as CI)
cargo clippy --all-features -- -D warnings -A clippy::too-many-arguments -A clippy::type-complexity

# Build all examples
cargo build --examples

# Test no-std build
cargo build --no-default-features
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
