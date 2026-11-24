# Changelog & Release Guide

This document describes how to maintain [CHANGELOG.md](file:///Users/jitera/Playground/jev-switch/CHANGELOG.md), write clean release notes, bump versions, and publish releases for `jev-switch` (`jev-code`, `jev-router`, `jev-claude`).

---

## 1. Principles

1. **Written for Humans**: Changelog entries are for developers and end-users, not machines. Do not dump raw git commits or internal review round labels (e.g., avoid `"Round 10 fixes"`). Explain _what_ changed, _why_ it matters, and _how_ to use it.
2. **Single Source of Truth**: The root [CHANGELOG.md](file:///Users/jitera/Playground/jev-switch/CHANGELOG.md) is the canonical record of all changes.
3. **Keep `Unreleased` Up to Date**: Every PR, feature, or bug fix must add a bullet under the `## Unreleased` section at the top of [CHANGELOG.md](file:///Users/jitera/Playground/jev-switch/CHANGELOG.md) when merged.
4. **Semantic Versioning (SemVer)**:
   - **Patch (`x.y.Z`)**: Backward-compatible bug fixes and security patches.
   - **Minor (`x.Y.0`)**: Backward-compatible new features, commands, or provider additions.
   - **Major (`X.0.0`)**: Breaking changes to configuration, command syntax, or routing behavior.

---

## 2. Formatting Standards

### Heading Hierarchy

```markdown
# Changelog

## Unreleased

- ...

## 1.4.0

Summary subtitle or theme (optional).

### Added

- **Bold feature title.** Clear description of what the feature does, flags to enable/disable it, and default behavior.

### Fixed

- Description of the bug and how it was resolved.

### Security

- Description of security tightening or vulnerability fixes.
```

### Writing Style

- **Lead with bold title**: Start each item with `**<Summary>.**` so readers can scan quickly.
- **Active voice & present tense**: Write `"Messages typed during a turn reach the running turn"` instead of `"Added feature where messages would be received"`.
- **State defaults & configuration**: If a behavior changed or can be configured via environment variables, state the variable name explicitly (e.g., `JEV_DELEGATE=off`).
- **Cross-platform notes**: Note platform-specific considerations (macOS, Linux, Windows) when relevant.

---

## 3. Step-by-Step Release Workflow

When cutting a new release (e.g., releasing `1.4.0`):

### Step 1: Promote `Unreleased` to the New Version

1. Open [CHANGELOG.md](file:///Users/jitera/Playground/jev-switch/CHANGELOG.md).
2. Create a new heading under `## Unreleased` with the target version:

   ```markdown
   ## Unreleased

   ## 1.4.0
   ```

3. Move the relevant items from `## Unreleased` down into the `## 1.4.0` section.
4. Add a short introductory sentence summarizing the release theme if applicable.

### Step 2: Bump the Version in `Cargo.toml` & `Cargo.lock`

1. Update `version` in [Cargo.toml](file:///Users/jitera/Playground/jev-switch/Cargo.toml):
   ```toml
   [package]
   name = "jev-router"
   version = "1.4.0"
   ```
2. Run `cargo check` to update `Cargo.lock`:
   ```bash
   cargo check
   ```

### Step 3: Run Verification

Ensure all library tests and end-to-end PTY suites pass:

```bash
cargo test --all-targets
python3 -I scripts/test_pty_ux.py
```

### Step 4: Git Commit

Commit with a clear, conventional summary:

```bash
git commit -am "jev-code 1.4.0: inline steering, delegate tool, and trimmed hints"
```

> [!IMPORTANT]
> The commit subject line should name the version and its primary highlight. Follow with a detailed body explaining the major changes.

### Step 5: Git Tag (Avoid Bare Tags!)

> [!WARNING]
> **Never create a bare tag annotation** like `git tag -a v1.4.0 -m "jev-code 1.4.0"`.
> GitHub's Tags page (`/tags`) displays the tag annotation message. A generic message makes the tags view look empty and repetitive.

**Always include release highlights in the tag message**:

```bash
git tag -a v1.4.0 -m "jev-code 1.4.0: inline steering, delegate tool, and trimmed hints"
```

### Step 6: Push Commit & Tag to Origin

Push the commit and the new tag by name (not `--tags`, which also pushes local tags that differ from the remote):

```bash
git push origin main v1.4.0
```

### Step 7: The Release Is Published by CI

Pushing the tag starts the `release` workflow (`.github/workflows/release.yml`). It checks that the tag matches the version in `Cargo.toml`, builds jev-code with `JEV_RELEASE_BUILD=1` for linux-x86_64, linux-aarch64, macos-x86_64, macos-aarch64 and windows-x86_64, packages each with `scripts/package-release.sh`, and publishes the GitHub release with `jev-code-v1.4.0-<platform>.tar.gz` and its `.sha256`. The release notes are this version's section of `CHANGELOG.md`. **Do not create the release by hand with `gh release create`**: a release made first has no binaries, and the workflow then fails to create its own.

Installed releases pick it up with `/update`, and `scripts/install-release.sh` installs it fresh. Watch the run with `gh run watch`.

### Step 8: Reinstall Locally & Verify

Reinstall to `~/.local/bin` and check system health:

```bash
bash scripts/install.sh
jev-code --version
jev-code doctor
```

---

## 4. Good vs. Bad Changelog Examples

| Bad (Avoid)                     | Good (Recommended)                                                                                                                                |
| ------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `- Fixed bug in turn.rs`        | `- **Turn steering cancellation**: A cancelled turn drops running CLI futures immediately, disarming background process groups cleanly.`          |
| `- Round 10 fixes`              | `- **Settings validation**: A project's \`.claude/settings.json\` is tracked by hash; changes require explicit re-approval via \`/hooks trust\`.` |
| `- Tag: v1.4.0`                 | `- **In-app updates (\`/update\`)\*\*: Added \`/update\` to fast-forward from \`origin/main\` without losing local uncommitted work.`             |
| `git tag -a v1.4.0 -m "v1.4.0"` | `git tag -a v1.4.0 -m "jev-code 1.4.0: inline steering, delegate MCP server, and trimmed hints"`                                                  |
