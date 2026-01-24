# GitHub Actions Workflows Reference

Quick reference guide for all workflows in this project.

## Workflow Overview

| Workflow | Trigger | Purpose | Secrets |
|----------|---------|---------|---------|
| CI | Push to main/develop, PR to main | Test, lint, build on multiple platforms | None |
| Release | GitHub release published | Publish to crates.io, build binaries | `CARGO_REGISTRY_TOKEN` |
| Security Audit | Daily, Cargo.toml/lock changes | CVE scanning, dependency audit | None |

---

## CI Workflow

**File:** `.github/workflows/ci.yml`

**Triggers:**
- `push` to `main` or `develop` branches
- `pull_request` to `main` branch

**Jobs:**
1. **Test Suite** - Runs on all platforms with stable and beta Rust
   - `cargo test --all-features`
   - `cargo test --doc`
   - `cargo test --no-default-features`

2. **Rustfmt** - Code formatting check
   - `cargo fmt -- --check`

3. **Clippy** - Linting with strict warnings
   - `cargo clippy --all-targets --all-features -- -D warnings`

4. **Documentation** - Build and check docs
   - `cargo doc --no-deps --all-features`

5. **Code Coverage** - Generate and upload coverage
   - `cargo tarpaulin --out Xml`

6. **MSRV** - Verify minimum supported Rust version
   - Parses `rust-version` from `Cargo.toml`
   - Runs `cargo check` with that version

7. **Build Release** - Build binaries for all targets
   - Linux (x86_64, musl, aarch64)
   - Allows failures (cross-compile may not work)

**Status Check:** ✅ Must pass before merging

**No secrets required.**

---

## Release Workflow

**File:** `.github/workflows/release.yml`

**Triggers:**
- GitHub release published (automatic)
- Manual workflow dispatch (run workflow button)

**Jobs:**

### 1. Verify Version
- Extracts version from git tag or manual input
- Compares with `Cargo.toml` version
- Fails if mismatch detected

### 2. Publish Crate
- Runs full test suite
- Builds documentation
- Publishes to crates.io using `CARGO_REGISTRY_TOKEN`

### 3. Build Binaries (Parallel)
Builds for 6 platforms simultaneously:
- `x86_64-unknown-linux-gnu`
- `x86_64-unknown-linux-musl`
- `aarch64-unknown-linux-gnu` (uses `cross`)
- `x86_64-apple-darwin` (macOS Intel)
- `aarch64-apple-darwin` (macOS ARM)
- `x86_64-pc-windows-msvc`

Creates `.tar.gz` archives (Unix) or `.zip` (Windows)

### 4. Create Checksums
Generates SHA256 checksums for all binaries

**Outputs:**
- ✅ Crate published to crates.io
- ✅ Binaries uploaded to GitHub release
- ✅ SHA256SUMS file uploaded

**Secrets Required:**
- `CARGO_REGISTRY_TOKEN` - crates.io API token

---

## Security Audit Workflow

**File:** `.github/workflows/security-audit.yml`

**Triggers:**
- Daily schedule (00:00 UTC)
- Push/PR with changes to `Cargo.toml` or `Cargo.lock`

**Jobs:**

### 1. Audit Dependencies
- Uses `rustsec/audit-check-action`
- Checks for known CVEs in dependencies
- Fails if vulnerabilities found

### 2. Cargo Deny
- Runs `cargo deny check all`
- Supply chain security checks
- Can fail on policy violations

### 3. Check Outdated
- Lists outdated dependencies
- Informational only (doesn't fail)

**Secrets Required:** None

**Note:** Failures don't block PRs (informational)

---

## How to Trigger Workflows

### CI Workflow (Automatic)
```bash
git push origin main
# or create a PR to main
```

### Release Workflow (Via GitHub UI)
1. Go to your repository
2. Click **Releases**
3. Click **Draft a new release**
4. Select tag matching your version (e.g., `v0.6.0`)
5. Click **Publish release**

### Release Workflow (Manual Dispatch)
1. Go to **Actions** tab
2. Click **Release** workflow
3. Click **Run workflow**
4. Enter version and run

---

## Environment Variables

Set in all workflows:
```yaml
CARGO_TERM_COLOR: always
RUST_BACKTRACE: 1
```

---

## Caching Strategy

Uses `Swatinem/rust-cache@v2` to cache:
- Dependencies (downloaded crates)
- Build artifacts
- Target directory

Cache keys by target for cross-platform builds.

---

## Branch Protection Rules

Recommended GitHub settings:
- Require status checks to pass: **CI**
- Require branches to be up to date
- Dismiss stale reviews when new commits pushed
- Require review from code owner (CODEOWNERS file)

---

## Artifact Cleanup

GitHub Actions artifacts are retained for:
- Public repos: 90 days
- Private repos: 400 days

Configure in Settings → Actions → General.

---

## Monitoring and Debugging

### View Workflow Runs
1. Go to **Actions** tab
2. Select workflow name
3. Click run to see details

### Check Status Badge
Add to README.md:
```markdown
[![CI](https://github.com/erichutchins/fstsed/actions/workflows/ci.yml/badge.svg)](https://github.com/erichutchins/fstsed/actions)
```

### Skip Workflow for a Commit
Add `[skip ci]` to commit message:
```bash
git commit -m "docs: update readme [skip ci]"
```

---

## Troubleshooting Quick Links

| Issue | Solution |
|-------|----------|
| Secret not found | Add `CARGO_REGISTRY_TOKEN` to Settings → Secrets |
| Version mismatch | Ensure tag `v0.6.0` matches Cargo.toml `0.6.0` |
| Tests fail on Windows | Use `std::path::Path` for cross-platform paths |
| Build times too long | Reduce matrix (fewer OS/Rust versions) |
| Cross-compile fails | Expected on non-native architectures; uses `cross` tool |
| crates.io publish fails | Check token, version not already published |
| Codecov not uploading | Optional; set `CODECOV_TOKEN` if desired |

---

## Performance Tips

1. **Use cache key strategy**
   - Separate caches by target
   - Cache between runs

2. **Parallel jobs**
   - Binary builds run in parallel
   - Reduces overall workflow time

3. **Skip on format-only changes**
   - Add path filters to workflows
   - Skip linting for doc-only changes

4. **Conditional jobs**
   - Use `if:` conditions
   - Skip cross-compilation in PRs

---

## Security Best Practices

1. **Never hardcode secrets** in workflow files
2. **Use repository secrets** for sensitive values
3. **Limit token scope** - use least-privilege tokens
4. **Rotate tokens regularly** - refresh crates.io token annually
5. **Audit workflow access** - review secret usage in logs
6. **Review PR workflows** - be careful with external contributions

---

## Workflow Dependencies

```
┌─────────────────────────────────────────────┐
│ GitHub Release Published                    │
└──────────────┬──────────────────────────────┘
               │
               ├─→ verify-version
               │   ├─→ publish-crate
               │   └─→ build-binaries (×6) parallel
               │       └─→ create-checksums
               │
               └─→ (upload to GitHub release)
```

All publish jobs depend on version verification passing.

---

## Common Workflow Patterns

### Test Before Release
```bash
# On main branch
cargo test --all-features
cargo fmt --check
cargo clippy -- -D warnings

# Wait for CI to pass, then release
git tag v0.6.0
git push origin v0.6.0
```

### Emergency Hotfix Release
```bash
# On main, critical fix only
cargo test --all-features
git commit -m "fix: critical bug"
git push origin main
# Wait for CI
git tag v0.5.1
git push origin v0.5.1
# Create release on GitHub
```

### Testing Feature Branch
```bash
git push origin feature/my-feature
# CI runs automatically on PR
# Address any failures
# Rebase and merge when ready
```

---

## Status Codes

Workflow status meanings:
- 🟢 **Success** - All jobs passed
- 🟡 **In Progress** - Jobs still running
- 🔴 **Failed** - One or more jobs failed
- ⚫ **Cancelled** - Manually cancelled
- ⚪ **Skipped** - Conditions not met or step skipped

---

## Useful Commands

```bash
# Test locally before pushing
cargo test --all-features
cargo fmt --check
cargo clippy -- -D warnings
cargo doc --no-deps

# Prepare a release
git tag -a v0.6.0 -m "Release 0.6.0"
git push origin v0.6.0

# Check workflows syntax (locally)
# Using GitHub CLI:
gh workflow list
gh workflow view ci.yml

# View recent runs
gh run list --workflow=ci.yml

# Re-run a failed workflow
gh run rerun RUN_ID --failed
```

---

## Related Documentation

- [SETUP.md](.github/SETUP.md) - Initial setup guide
- [RELEASE_GUIDE.md](.github/RELEASE_GUIDE.md) - Detailed release process
- [CODEOWNERS](.github/CODEOWNERS) - Code ownership rules

---

## Support

For help:
1. Check GitHub Actions logs
2. Review relevant documentation file
3. Search [GitHub Discussions](https://github.com/erichutchins/fstsed/discussions)
4. Open a [GitHub Issue](https://github.com/erichutchins/fstsed/issues)
