# GitHub Actions & CI/CD

This directory contains GitHub Actions workflows and documentation for automating testing, building, and publishing `fstsed`.

## 📋 Quick Links

- **[SETUP.md](SETUP.md)** - Initial setup guide (start here!)
- **[WORKFLOWS.md](WORKFLOWS.md)** - Quick reference for all workflows
- **[RELEASE_GUIDE.md](RELEASE_GUIDE.md)** - Detailed release process
- **[CODEOWNERS](CODEOWNERS)** - Code ownership and review rules

## 🚀 What's Included

### Three Automated Workflows

#### 1. **CI Workflow** (`.github/workflows/ci.yml`)
Runs on every push and pull request.

**Tests:**
- Multi-platform testing (Linux, macOS, Windows)
- Multiple Rust versions (stable, beta)
- Code formatting (rustfmt)
- Linting (clippy with strict warnings)
- Documentation build and testing
- Code coverage analysis
- MSRV verification
- Release binary builds

**Status:** ✅ Required to pass before merging

---

#### 2. **Release Workflow** (`.github/workflows/release.yml`)
Triggered when you publish a GitHub release.

**Publishes:**
- Crate to crates.io
- Binaries for 6 platforms:
  - Linux (x86_64, x86_64-musl, aarch64)
  - macOS (x86_64, aarch64)
  - Windows (x86_64-msvc)
- SHA256 checksums

**Requires:** `CARGO_REGISTRY_TOKEN` secret configured

---

#### 3. **Security Audit Workflow** (`.github/workflows/security-audit.yml`)
Runs daily and on dependency changes.

**Checks:**
- CVE scanning (rustsec)
- Supply chain security (cargo-deny)
- Outdated dependency detection

**Status:** ℹ️ Informational (doesn't block merges)

---

## 🔑 One-Time Setup

### Add Your crates.io Token

Required for publishing releases to crates.io.

1. Get your token: https://crates.io/me/tokens
2. Go to GitHub: Settings → Secrets and variables → Actions
3. Create new secret: `CARGO_REGISTRY_TOKEN`
4. Paste your crates.io token
5. Save

**That's it!** All workflows are now ready to use.

---

## 📦 Making a Release

### Simple 4-Step Process

```bash
# 1. Update version in Cargo.toml
# version = "0.6.0"

git add Cargo.toml
git commit -m "chore: bump version to 0.6.0"
git push origin main

# 2. Wait for CI to pass (check Actions tab)

# 3. Create git tag
git tag -a v0.6.0 -m "Release 0.6.0"
git push origin v0.6.0

# 4. Create GitHub release
# Go to: https://github.com/erichutchins/fstsed/releases
# Click "Draft a new release"
# Select tag v0.6.0
# Add release notes
# Click "Publish release"
```

**That's it!** The workflow automatically:
- Verifies version consistency
- Publishes to crates.io
- Builds binaries for all platforms
- Uploads artifacts to GitHub release

---

## ✅ Pre-Release Checklist

Before releasing:

```bash
# Run these locally
cargo test --all-features
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo doc --no-deps --all-features

# Wait for CI to pass on main
# Then create your release tag
```

---

## 🔍 Monitoring Workflows

### View Status

1. Go to **Actions** tab on GitHub
2. Click workflow name to see all runs
3. Click specific run for detailed logs

### Status Badge

Add to your README:

```markdown
[![CI](https://github.com/erichutchins/fstsed/actions/workflows/ci.yml/badge.svg)](https://github.com/erichutchins/fstsed/actions)
```

---

## 📚 Documentation Files

### SETUP.md
Complete setup guide covering:
- crates.io token configuration
- Codecov integration (optional)
- Workflow customization
- Troubleshooting guide

**Read this if:** You're setting up for the first time

### WORKFLOWS.md
Quick reference guide with:
- Workflow summary table
- Trigger conditions
- Job descriptions
- Common tasks
- Troubleshooting quick links

**Read this if:** You need a quick lookup

### RELEASE_GUIDE.md
Detailed release process covering:
- Step-by-step release instructions
- Version numbering strategy
- Release artifacts explanation
- Performance tips
- Security considerations

**Read this if:** You're making a release

### CODEOWNERS
Specifies code ownership for PR reviews.

**Edit this if:** You want to change review requirements

---

## 🔧 Customization

### Adjust Test Platforms

Edit `.github/workflows/ci.yml`:

```yaml
matrix:
  os: [ubuntu-latest, macos-latest, windows-latest]
  rust: [stable, beta]
```

Remove entries you don't need to speed up CI.

### Change Schedule for Security Audits

Edit `.github/workflows/security-audit.yml`:

```yaml
schedule:
  - cron: '0 0 * * *'  # Daily at midnight UTC
  # or: '0 0 * * 0'    # Weekly on Sundays
  # or: '0 12 * * *'   # Daily at noon UTC
```

### Add More Binary Targets

Edit `.github/workflows/release.yml` to add platforms:

```yaml
matrix:
  include:
    # ... existing targets ...
    - os: ubuntu-latest
      target: arm-unknown-linux-gnueabihf  # Add ARM support
      use_cross: true
```

---

## 🚨 Troubleshooting

### "Version mismatch" error

Ensure:
- Git tag: `v0.6.0` (with `v` prefix)
- Cargo.toml: `version = "0.6.0"` (without `v`)
- They must match exactly

### "Secret not found" error

Add `CARGO_REGISTRY_TOKEN` to GitHub Settings:
- Settings → Secrets and variables → Actions
- New secret: `CARGO_REGISTRY_TOKEN`
- Value: Your crates.io API token

### Tests pass locally but fail in CI

Check for:
- Platform-specific code (use `#[cfg(target_os = "...")]`)
- Environment-dependent behavior
- Race conditions
- File path issues (use `std::path::Path`)

### More help?

See detailed troubleshooting in:
- [SETUP.md](SETUP.md#troubleshooting-checklist) - Setup issues
- [WORKFLOWS.md](WORKFLOWS.md#troubleshooting-quick-links) - Workflow issues

---

## 📊 Workflow Matrix

| Workflow | Trigger | Status | Secrets |
|----------|---------|--------|---------|
| **CI** | Push/PR | Required ✅ | None |
| **Release** | GitHub release | Auto | `CARGO_REGISTRY_TOKEN` |
| **Security** | Daily/On changes | Info ℹ️ | None |

---

## 🔒 Security

### Best Practices

✅ **Do:**
- Store tokens in GitHub Secrets only
- Use least-privilege API tokens
- Rotate crates.io tokens periodically
- Review workflow permissions

❌ **Don't:**
- Commit secrets to version control
- Share tokens in issues or PRs
- Log secrets in workflow output
- Use permanent tokens

### Audit Logging

GitHub automatically logs:
- Who triggered workflows
- When they ran
- What secrets were accessed
- Deployment results

---

## 🎯 Key Features

✨ **Multi-platform builds** - Binaries for Linux, macOS, Windows

✨ **Cross-compilation** - ARM64 support via `cross` tool

✨ **Automated publishing** - Single command to release

✨ **Security scanning** - Daily CVE checks

✨ **Code coverage** - Track test coverage over time

✨ **Fast CI** - Caching for quick feedback

✨ **Version verification** - Prevents version mismatches

✨ **SHA256 checksums** - Binary integrity verification

---

## ⚡ Quick Commands

```bash
# Test everything locally before releasing
cargo test --all-features && cargo fmt --check && cargo clippy -- -D warnings

# Create a release tag
git tag -a v0.6.0 -m "Release 0.6.0"
git push origin v0.6.0

# View workflow status
gh run list --workflow=ci.yml

# Re-run failed workflow
gh run rerun RUN_ID --failed
```

---

## 📖 External Resources

- **GitHub Actions**: https://docs.github.com/actions
- **Rust Guide**: https://doc.rust-lang.org/
- **crates.io**: https://crates.io/
- **Semantic Versioning**: https://semver.org/

---

## ❓ FAQ

**Q: Do I need to set up anything?**
A: Yes, add your crates.io token as a secret. See [SETUP.md](SETUP.md).

**Q: How do I make a release?**
A: Follow the 4-step process in [Making a Release](#-making-a-release) above.

**Q: Can I customize the workflows?**
A: Yes! See [Customization](#-customization) section.

**Q: What if something fails?**
A: Check [Troubleshooting](#-troubleshooting) or the detailed guides.

**Q: Do I need to build binaries locally?**
A: No! The release workflow builds them for you.

**Q: How often should I update dependencies?**
A: Regularly. The security audit workflow alerts you to issues.

---

## 📝 Summary

This setup provides:

1. ✅ **Continuous Integration** - Tests run automatically
2. ✅ **Automated Publishing** - Release in 4 simple steps
3. ✅ **Security Scanning** - Daily vulnerability checks
4. ✅ **Multi-platform Binaries** - Built automatically
5. ✅ **Documentation** - Everything is documented here

**Get started:** Read [SETUP.md](SETUP.md) to configure your crates.io token.

---

**Last Updated:** January 2024
**Maintained by:** @erichutchins
