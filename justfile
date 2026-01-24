set shell := ["zsh", "-cu"]

# Automatically extract version from Cargo.toml
version := `cargo pkgid --quiet | cut -d# -f2`

# List available commands
default:
    @just --list

# --- Git / Branch Management ---

# Update current branch by rebasing on top of the latest main
sync-main:
    git config pull.rebase true
    git fetch origin main
    git rebase origin/main
    @echo "Success: Branch updated and rebased on origin/main"

# Force push current branch after a rebase (careful!)
push-force:
    git push --force-with-lease

# Shortcut to sync and then force push (common flow for develop branch)
publish-develop: sync-main push-force

# --- Development ---

# Run all tests, including doc tests
test:
    cargo test --all-features

# Run clippy and check formatting
lint:
    cargo fmt -- --check
    cargo clippy --all-targets --all-features -- -D warnings

# Build a release binary for the current platform
build:
    cargo build --release

# --- Release / Minting ---

# Prepare for a release by checking everything
pre-release: lint test build

# Mint a new release tag using the version in Cargo.toml
mint-release: pre-release
    @echo "Minting release v{{version}}..."
    @git tag -a v{{version}} -m "Release v{{version}}"
    @git push origin v{{version}}
    @echo "Success: Release v{{version}} minted and pushed."
