# GitHub Actions Setup Guide for fstsed

This guide walks you through setting up and configuring GitHub Actions for automated Rust compilation, testing, and publishing to crates.io.

## Quick Start

### 1. Prerequisites

- A GitHub repository with your Rust project
- A crates.io account (https://crates.io)
- Your crates.io API token

### 2. Add Your crates.io Token

**This is required for publishing to crates.io.**

1. Go to your crates.io account settings: https://crates.io/me
2. Navigate to API Tokens section
3. Generate a new token (or use existing one)
4. Go to your GitHub repository
5. Settings → Secrets and variables → Actions
6. Click "New repository secret"
7. Name: `CARGO_REGISTRY_TOKEN`
8. Value: Paste your crates.io API token
9. Click "Add secret"

## Workflow Files

Your `.github/workflows/` directory contains three workflows:

### CI Workflow (`ci.yml`)

**Purpose:** Continuous integration - runs on every push and pull request

**Runs:**
- Tests on Linux, macOS, and Windows
- Tests with stable and beta Rust versions
- Code formatting checks (rustfmt)
- Linting checks (clippy)
- Documentation builds
- Code coverage analysis
- MSRV verification
- Release binary builds

**Triggers:**
- Push to `main` or `develop` branches
- Pull requests to `main`

**Status:** Required to pass before merging PRs

### Release Workflow (`release.yml`)

**Purpose:** Publishes your crate to crates.io and creates binary releases

**Runs:**
- Version consistency verification
- Full test suite
- Publishing to crates.io
- Building binaries for multiple platforms
- Creating SHA256 checksums
- Uploading binaries to GitHub releases

**Platforms Built:**
- x86_64-unknown-linux-gnu (Ubuntu/glibc)
- x86_64-unknown-linux-musl (Alpine/musl)
- aarch64-unknown-linux-gnu (ARM64 Linux)
- x86_64-apple-darwin (macOS Intel)
- aarch64-apple-darwin (macOS ARM)
- x86_64-pc-windows-msvc (Windows)

**Triggers:**
- GitHub release published (automatic)
- Manual workflow dispatch with version input

### Security Audit Workflow (`security-audit.yml`)

**Purpose:** Checks for vulnerable and outdated dependencies

**Checks:**
- rustsec audit (CVE database)
- cargo-deny supply chain security
- Outdated dependency detection

**Triggers:**
- Daily schedule (00:00 UTC)
- Push/PR with changes to Cargo.toml or Cargo.lock

**Note:** Failures don't block merges (informational)

## Making Your First Release

### Step 1: Update Version

Edit `Cargo.toml` and update the version:

```toml
[package]
name = "fstsed"
version = "0.6.0"  # ← Update this
```

### Step 2: Prepare Changes

```bash
# Commit version change
git add Cargo.toml
git commit -m "chore: bump version to 0.6.0"
git push origin main
```

### Step 3: Create Git Tag

```bash
# Create annotated tag
git tag -a v0.6.0 -m "Release version 0.6.0"

# Push tag to GitHub
git push origin v0.6.0
```

**Important:** Tag format must be `v` followed by version number (e.g., `v0.6.0`)

### Step 4: Create GitHub Release

1. Go to your GitHub repository
2. Click **Releases** in the sidebar
3. Click **Draft a new release**
4. Select your tag (v0.6.0)
5. Fill in release title and description
6. Check "Set as the latest release"
7. Click **Publish release**

### Step 5: Monitor the Workflow

1. Go to **Actions** tab
2. Watch the Release workflow run
3. It will:
   - Verify version consistency
   - Run tests
   - Publish to crates.io
   - Build and upload binaries

The workflow typically completes in 5-15 minutes.

## Version Numbering

Follow Semantic Versioning (semver):

- **MAJOR.MINOR.PATCH** (e.g., 1.0.0)
- **MAJOR**: Breaking changes
- **MINOR**: New features (backward compatible)
- **PATCH**: Bug fixes

### Pre-release Versions

For alpha, beta, or release candidate versions:

```toml
version = "0.6.0-alpha.1"
version = "0.6.0-beta.2"
version = "0.6.0-rc.1"
```

Create tags similarly:
```bash
git tag -a v0.6.0-beta.1 -m "Beta 1"
git push origin v0.6.0-beta.1
```

When creating the GitHub release, check **Set as pre-release**.

## Continuous Integration Details

### Test Matrix

The CI workflow tests against:

| OS | Rust Versions |
|---|---|
| Ubuntu (Latest) | stable, beta |
| macOS (Latest) | stable, beta |
| Windows (Latest) | stable, beta |

This ensures your code works across platforms and Rust versions.

### Code Quality Checks

1. **Formatting**: `cargo fmt -- --check`
   - Ensures consistent code style

2. **Linting**: `cargo clippy --all-targets --all-features -- -D warnings`
   - Catches common mistakes and anti-patterns

3. **Tests**: Full test suite on all platforms
   - Includes unit tests and integration tests

4. **Documentation**: Doc comment examples are tested
   - Ensures documentation is accurate

5. **Code Coverage**: Measured with tarpaulin
   - Uploaded to Codecov (optional integration)

### MSRV Verification

The Minimum Supported Rust Version (MSRV) is specified in `Cargo.toml`:

```toml
rust-version = "1.93"
```

The CI workflow verifies your code compiles with this version.

## Environment Variables

All workflows set these environment variables:

```yaml
CARGO_TERM_COLOR: always
RUST_BACKTRACE: 1
```

These enable colored output and full backtraces on panic.

## Caching

Workflows use `Swatinem/rust-cache@v2` to cache:
- Downloaded dependencies
- Compiled artifacts
- Target directory

This significantly speeds up CI runs.

## Security Considerations

### Secrets Management

- **Never** commit secrets to version control
- **Always** use GitHub Secrets for sensitive values
- Rotate crates.io tokens periodically
- Review secret access in workflow logs

### Safe Dependencies

The security audit workflow:
- Checks for known CVEs
- Identifies supply chain issues
- Detects outdated packages

Address any reported issues before merging to main.

### Permissions

Workflows have minimal permissions:
- Read access to repository code
- Write access only for releases and comments
- Token-based authentication for crates.io

## Customization

### Modifying Test Platforms

Edit `.github/workflows/ci.yml` to customize the test matrix:

```yaml
strategy:
  matrix:
    os: [ubuntu-latest, macos-latest, windows-latest]  # Add/remove as needed
    rust: [stable, beta]  # Add nightly for cutting-edge testing
```

### Disabling Certain Checks

To temporarily disable a job, comment it out or delete it. For example:

```yaml
# coverage:
#   name: Code Coverage
#   ...
```

### Adding Custom Build Targets

In `ci.yml`, add targets to the build matrix:

```yaml
target:
  - x86_64-unknown-linux-gnu
  - aarch64-unknown-linux-gnu
  - arm-unknown-linux-gnueabihf  # Add custom targets
```

### Adjusting Timeouts

For long-running tasks, increase timeouts:

```yaml
- name: Run slow tests
  run: cargo test --release
  timeout-minutes: 30  # Add this line
```

## Troubleshooting

### "Secret not found" Error

**Problem:** `CARGO_REGISTRY_TOKEN` is missing

**Solution:**
1. Go to Settings → Secrets and variables → Actions
2. Add `CARGO_REGISTRY_TOKEN` with your crates.io token
3. Re-run the workflow

### "Version mismatch" Error

**Problem:** Git tag doesn't match Cargo.toml version

**Solution:**
- Tag should be: `v0.6.0` (with `v` prefix)
- Cargo.toml should have: `version = "0.6.0"` (without `v`)

Example:
```bash
# Good
git tag v0.6.0  # matches version in Cargo.toml

# Bad
git tag 0.6.0   # missing v prefix
git tag v0.6    # doesn't match 0.6.0
```

### Tests Pass Locally but Fail in CI

**Possible causes:**
1. Platform-specific code (use `#[cfg(target_os = "...")]`)
2. Race conditions (use proper synchronization)
3. File path issues (use forward slashes or `Path` APIs)
4. Environment-dependent code (mock in tests)

### Workflow Stuck on "pending"

**Problem:** Workflow hasn't started after 1+ hours

**Solution:**
1. Check GitHub Status: https://www.githubstatus.com
2. Cancel and re-run the workflow
3. Check workflow file syntax for errors

### Binary Build Failed for Specific Target

**Problem:** Cross-compilation failed for ARM/other target

**Possible solutions:**
1. Some targets may fail on non-native CI runners (expected)
2. Install `cross` tool for cross-compilation
3. Check target-specific dependencies in `Cargo.toml`

## Best Practices

### 1. Test Before Release

Always ensure CI passes on main before creating a release:

```bash
git log --oneline -1  # Verify latest commit
# Check Actions tab to confirm all checks pass
```

### 2. Use Semantic Versioning

- Increment MAJOR for breaking changes
- Increment MINOR for new features
- Increment PATCH for bug fixes

### 3. Write Good Release Notes

Include in GitHub release description:
- Major changes and features
- Bug fixes
- Breaking changes (if any)
- Known issues
- Acknowledgments

### 4. Keep Dependencies Updated

Regularly run:
```bash
cargo update
cargo audit
cargo outdated
```

The security audit workflow checks this automatically.

### 5. Document Breaking Changes

When making MAJOR version bumps, clearly document:
- What changed
- How to migrate
- Why the change was necessary

## Advanced Configuration

### Adding Codecov Integration

1. Sign up at https://codecov.io
2. Enable your GitHub repository
3. Add `CODECOV_TOKEN` secret to GitHub (optional)
4. The workflow automatically uploads coverage

### Using Specific Rust Versions

To test specific nightly versions:

```yaml
- uses: dtolnay/rust-toolchain@master
  with:
    toolchain: nightly-2024-01-01
```

### Matrix Complexity

For complex testing scenarios:

```yaml
strategy:
  matrix:
    include:
      - os: ubuntu-latest
        rust: stable
        target: x86_64-unknown-linux-gnu
      - os: ubuntu-latest
        rust: stable
        target: aarch64-unknown-linux-gnu
        use_cross: true
```

## Support and Resources

- **GitHub Docs**: https://docs.github.com/en/actions
- **Rust Docs**: https://doc.rust-lang.org/
- **crates.io**: https://crates.io/
- **dtolnay Toolchain**: https://github.com/dtolnay/rust-toolchain

## Next Steps

1. ✅ Add `CARGO_REGISTRY_TOKEN` secret
2. ✅ Verify CI passes on your current code
3. ✅ Create your first release following "Making Your First Release"
4. ✅ Monitor the workflows in the Actions tab
5. ✅ Celebrate your automated release pipeline! 🎉
