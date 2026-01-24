# Release Guide

This guide explains how to use the GitHub Actions workflows set up for fstsed.

## Workflows Overview

### 1. CI Workflow (`ci.yml`)

Runs automatically on:
- Every push to `main` or `develop` branches
- Every pull request to `main`

**What it does:**
- Runs tests on Ubuntu, macOS, and Windows
- Tests against stable and beta Rust versions
- Checks code formatting with `rustfmt`
- Lints code with `clippy`
- Builds documentation
- Generates code coverage reports
- Verifies MSRV (Minimum Supported Rust Version)
- Builds release binaries for multiple targets

**Status:** All checks must pass before merging to main.

### 2. Release Workflow (`release.yml`)

Triggers automatically when you publish a GitHub release, or manually via workflow dispatch.

**What it does:**
- Verifies version consistency between tag and Cargo.toml
- Runs full test suite
- Publishes the crate to crates.io
- Builds binaries for all supported platforms:
  - x86_64-unknown-linux-gnu
  - aarch64-unknown-linux-gnu (requires cross-compilation)
  - x86_64-apple-darwin (macOS Intel)
  - aarch64-apple-darwin (macOS ARM)
  - x86_64-pc-windows-msvc (Windows)
- Uploads binaries as GitHub release assets

### 3. Security Audit Workflow (`security-audit.yml`)

Runs automatically:
- On schedule (daily at midnight UTC)
- When Cargo.toml or Cargo.lock changes
- On every pull request

**What it does:**
- Audits dependencies for known vulnerabilities (rustsec)
- Runs cargo-deny checks for supply chain security
- Reports on outdated dependencies

## Publishing a Release

### Prerequisites

1. **Set up crates.io token:**
   - Go to your crates.io account settings
   - Create an API token
   - In GitHub repo settings → Secrets and variables → Actions
   - Add a new secret: `CARGO_REGISTRY_TOKEN` with your crates.io token

2. **Verify your setup:**
   - All tests pass locally and in CI
   - Cargo.toml version matches the version you want to release
   - CHANGELOG is updated

### Release Steps

#### Option 1: Using GitHub Web Interface (Recommended)

1. **Update Cargo.toml version:**
   ```toml
   [package]
   version = "0.5.1"  # Update this
   ```

2. **Update CHANGELOG (if you have one)**

3. **Commit and push changes:**
   ```bash
   git add Cargo.toml CHANGELOG.md
   git commit -m "chore: bump version to 0.5.1"
   git push origin main
   ```

4. **Wait for CI to pass** - Verify all tests pass on the main branch

5. **Create a GitHub Release:**
   - Go to your GitHub repo → Releases → Draft a new release
   - Tag version: `v0.5.1` (match Cargo.toml version with `v` prefix)
   - Release title: `v0.5.1` or `fstsed 0.5.1`
   - Description: Add release notes
   - Click "Publish release"

6. **Workflow automatically:**
   - Verifies version consistency
   - Runs tests
   - Publishes to crates.io
   - Builds binaries for all platforms
   - Uploads binaries as release assets

#### Option 2: Manual Workflow Dispatch

If you need to publish without creating a GitHub release:

1. Go to Actions → Release → Run workflow
2. Enter the version (e.g., `0.5.1`)
3. Click "Run workflow"

### Version Numbering

Follow semantic versioning:
- **MAJOR.MINOR.PATCH** (e.g., `0.5.1`)
- MAJOR: Breaking changes
- MINOR: New features (backward compatible)
- PATCH: Bug fixes

### What Gets Published

- **To crates.io:** The crate with all dependencies
- **To GitHub Releases:** 
  - `fstsed-v0.5.1-x86_64-unknown-linux-gnu.tar.gz`
  - `fstsed-v0.5.1-x86_64-apple-darwin.tar.gz`
  - `fstsed-v0.5.1-aarch64-apple-darwin.tar.gz`
  - `fstsed-v0.5.1-x86_64-pc-windows-msvc.zip`
  - `fstsed-v0.5.1-aarch64-unknown-linux-gnu.tar.gz`

## Monitoring Workflows

### View Workflow Status

1. Go to your GitHub repo → Actions
2. Click on the workflow name to see details
3. Click on a run to see logs for each job

### Common Issues

**Version Mismatch Error:**
- Ensure tag matches Cargo.toml version
- Tag format: `v0.5.1` (with `v` prefix)
- Cargo.toml: `version = "0.5.1"` (without `v`)

**Crates.io Publication Fails:**
- Verify `CARGO_REGISTRY_TOKEN` secret is set
- Token must have publish permissions
- Check for duplicate version (already published)
- Ensure Cargo.toml is valid

**Binary Build Fails:**
- Cross-compilation (aarch64-unknown-linux-gnu) requires `cross`
- The workflow auto-installs it if needed
- Can safely ignore cross-compilation failures for non-native targets

**Security Audit Failures:**
- Check rustsec advisory database
- Either update vulnerable dependency or ignore with cargo-deny config
- Add `.cargo/deny.toml` to repo if needed

## Maintenance

### Updating Dependencies

1. Run `cargo update` locally
2. Test thoroughly
3. Push to develop branch
4. Security audit will run automatically
5. Address any reported issues before merging to main

### Modifying Workflows

If you need to modify the workflows:

1. Make changes to `.github/workflows/*.yml`
2. Push to a branch and create PR
3. Changes take effect once merged to main
4. Test with workflow_dispatch if available

## Tips and Best Practices

- **Test before releasing:** Always ensure CI passes on main
- **Use semantic versioning:** Makes releases predictable
- **Keep CHANGELOG updated:** Users appreciate knowing what changed
- **Sign commits:** Consider GPG signing for releases
- **Automate patch releases:** For critical fixes, release quickly
- **Document breaking changes:** Clearly mark MAJOR version changes
- **Check crates.io yanking:** Rarely need it, but available if critical issues found

## Troubleshooting

### Workflow shows "pending" forever

GitHub Actions might be experiencing delays. Check GitHub Status page: https://www.githubstatus.com

### Published to crates.io but not showing up

- crates.io indexes asynchronously (usually <1 minute)
- Check: https://crates.io/crates/fstsed
- Verify version in Cargo.toml

### Need to re-run a failed workflow

1. Go to Actions → Failed workflow run
2. Click "Re-run failed jobs" or "Re-run all jobs"
3. Make sure to fix underlying issues first

### Want to skip a workflow for a commit

Add `[skip ci]` to your commit message:
```bash
git commit -m "docs: update readme [skip ci]"
```

## Related Files

- `.github/workflows/ci.yml` - Continuous integration
- `.github/workflows/release.yml` - Release automation
- `.github/workflows/security-audit.yml` - Dependency security
- `Cargo.toml` - Package configuration
- `Cargo.lock` - Dependency lock file

## Support

For issues with the workflows:
1. Check GitHub Actions logs
2. Search existing GitHub Issues
3. Create a new issue with workflow logs attached
