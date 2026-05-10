---
name: crafting-readme-files
description: >-
  Craft professional README.md files for GitHub open source projects.
  Generates hero sections, installation instructions, feature tables, and
  architecture diagrams. Use when creating or revising a README, documenting
  a CLI tool, library, or open source project, or when user asks about
  README structure, badges, or project documentation.
---

# Crafting README.md Files for GitHub

> **Core insight:** A README is a sales pitch, onboarding guide, and reference manual compressed into one document. Lead with value, prove with examples, document with precision.

## Why This Matters

Most READMEs fail because they:
- Bury the value proposition under installation steps
- Explain what the tool IS instead of what problem it SOLVES
- Lack concrete examples (abstract descriptions don't sell)
- Miss the "quick escape hatch" for impatient users (curl one-liner)
- Don't show how it compares to alternatives

Great READMEs convert scanners into users in under 60 seconds.

---

## THE EXACT PROMPT — README Revision

```
Read the current README.md and dramatically revise it following this structure:

1. Hero section: illustration + badges + one-liner description + curl install
2. TL;DR: "The Problem" + "The Solution" + "Why Use X?" feature table
3. Quick example showing the tool in action (5-10 commands)
4. Design philosophy (3-5 principles with explanations)
5. Comparison table vs alternatives
6. Installation (curl one-liner, package managers, from source)
7. Quick start (numbered steps, copy-paste ready)
8. Command reference (every command with examples)
9. Configuration (full config file example with comments)
10. Architecture diagram (ASCII art showing data flow)
11. Troubleshooting (common errors with fixes)
12. Limitations (honest about what it doesn't do)
13. FAQ (anticipate user questions)

Make it comprehensive but scannable. Use tables for comparisons.
Show, don't tell. Every claim should have a concrete example.
Use ultrathink.
```

---

## Golden Structure

```
1. HERO SECTION (above the fold)
   ├─ Illustration/logo (centered)
   ├─ Badges (CI, license, version)
   ├─ One-liner description
   └─ Quick install (curl | bash)

2. TL;DR (sell the value)
   ├─ The Problem (pain point)
   ├─ The Solution (what this does)
   └─ Why Use X? (feature table)

3. QUICK EXAMPLE (prove it works)
   └─ 5-10 commands showing core workflow

4. REFERENCE SECTIONS
   ├─ Design Philosophy
   ├─ Comparison vs Alternatives
   ├─ Installation (multiple paths)
   ├─ Quick Start
   ├─ Commands
   ├─ Configuration
   └─ Architecture

5. SUPPORT SECTIONS
   ├─ Troubleshooting
   ├─ Limitations
   ├─ FAQ
   ├─ Contributing
   └─ License
```

---

## Section Templates

### Hero Section

```markdown
# tool-name

<div align="center">
  <img src="illustration.webp" alt="tool-name - One-line description">
</div>

<div align="center">

[![CI](https://github.com/user/repo/actions/workflows/ci.yml/badge.svg)](https://github.com/user/repo/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)

</div>

One-sentence description of what this tool does and its key differentiator.

<div align="center">
<h3>Quick Install</h3>

```bash
curl -fsSL https://raw.githubusercontent.com/user/repo/main/install.sh | bash
```

**Or build from source:**

```bash
cargo install --git https://github.com/user/repo.git
```

</div>
```

### TL;DR Section

```markdown
## TL;DR

**The Problem**: [Specific pain point in 1-2 sentences. Be concrete.]

**The Solution**: [What this tool does to solve it. Action-oriented.]

### Why Use tool-name?

| Feature | What It Does |
|---------|--------------|
| **Feature 1** | Concrete benefit, not abstract capability |
| **Feature 2** | Another specific value proposition |
| **Feature 3** | Quantify when possible (e.g., "<10ms search") |
```

### Quick Example

```markdown
### Quick Example

```bash
# Initialize (one-time setup)
$ tool init

# Core operation
$ tool do-thing --flag value

# See results
$ tool show results

# The killer feature
$ tool magic --auto
```
```

### Comparison Table

```markdown
## How tool-name Compares

| Feature | tool-name | Alternative A | Alternative B | Manual |
|---------|-----------|---------------|---------------|--------|
| Feature 1 | ✅ Full support | ⚠️ Partial | ❌ None | ❌ |
| Feature 2 | ✅ <10ms | 🐢 ~500ms | ✅ Fast | N/A |
| Setup time | ✅ ~10 seconds | ❌ Hours | ⚠️ Minutes | ❌ |

**When to use tool-name:**
- Bullet point of ideal use case
- Another use case

**When tool-name might not be ideal:**
- Honest limitation
- Another case where alternatives win
```

### Installation Section

```markdown
## Installation

### Quick Install (Recommended)

```bash
curl -fsSL https://raw.githubusercontent.com/user/repo/main/install.sh | bash
```

**With options:**

```bash
# Auto-update PATH
curl -fsSL https://... | bash -s -- --easy-mode

# Specific version
curl -fsSL https://... | bash -s -- --version v1.0.0

# System-wide (requires sudo)
curl -fsSL https://... | sudo bash -s -- --system
```

### Package Managers

```bash
# macOS/Linux (Homebrew)
brew install user/tap/tool

# Windows (Scoop)
scoop bucket add user https://github.com/user/scoop-bucket
scoop install tool
```

### From Source

```bash
git clone https://github.com/user/repo.git
cd repo
cargo build --release
cp target/release/tool ~/.local/bin/
```
```

### Command Reference Pattern

```markdown
## Commands

Global flags available on all commands:

```bash
--verbose       # Increase logging
--quiet         # Suppress non-error output
--format json   # Machine-readable output
```

### `tool command`

Brief description of what this command does.

```bash
tool command                    # Basic usage
tool command --flag value       # With options
tool command --help             # See all options
```
```

### Architecture Diagram

```markdown
## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         Input Layer                              │
│   (files, API calls, user commands)                             │
└─────────────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────────────┐
│                      Processing Layer                            │
│   Component A → Component B → Component C                        │
└─────────────────────────────────────────────────────────────────┘
                            │
        ┌───────────────────┼───────────────────┐
        ▼                   ▼                   ▼
┌──────────────────┐ ┌──────────────────┐ ┌──────────────────┐
│ Storage A        │ │ Storage B        │ │ Output           │
│ - Detail 1       │ │ - Detail 1       │ │ - Format 1       │
│ - Detail 2       │ │ - Detail 2       │ │ - Format 2       │
└──────────────────┘ └──────────────────┘ └──────────────────┘
```
```

### Troubleshooting Pattern

```markdown
## Troubleshooting

### "Error message here"

```bash
# Solution
command to fix it
```

### "Another common error"

Explanation of why this happens and how to fix it.

```bash
# Check the state
diagnostic command

# Fix it
fix command
```
```

### Limitations Section

```markdown
## Limitations

### What tool-name Doesn't Do (Yet)

- **Limitation 1**: Brief explanation, workaround if any
- **Limitation 2**: Why this is out of scope

### Known Limitations

| Capability | Current State | Planned |
|------------|---------------|---------|
| Feature X | ❌ Not supported | v2.0 |
| Feature Y | ⚠️ Partial | Improving |
```

### FAQ Pattern

```markdown
## FAQ

### Why "tool-name"?

Brief etymology or meaning.

### Is my data safe?

Yes/No with explanation. Privacy guarantees.

### Does it work with X?

Compatibility information.

### How do I [common task]?

```bash
# Command to accomplish it
tool do-thing
```
```

---

## Critical Rules

1. **Lead with value, not installation** — TL;DR before Quick Start
2. **Curl one-liner above the fold** — Impatient users escape hatch
3. **Every feature claim needs an example** — Show, don't tell
4. **Comparison tables beat prose** — Scannable > readable
5. **Be honest about limitations** — Builds trust, saves support time
6. **Multiple installation paths** — curl, package manager, source
7. **Architecture diagrams for complex tools** — ASCII art is fine
8. **Troubleshooting section is mandatory** — Top 5 errors with fixes

---

## Anti-Patterns (Avoid)

| Anti-Pattern | Why Bad | Fix |
|--------------|---------|-----|
| Installation-first README | Buries value proposition | Lead with TL;DR |
| "This is a tool that..." | Passive, abstract | "Solves X by doing Y" |
| Screenshot-heavy | Breaks, doesn't copy-paste | ASCII + code blocks |
| No examples | Abstract claims don't sell | Every feature → example |
| Hiding limitations | Users discover painfully | Honest Limitations section |
| Single install method | Alienates users | curl + pkg manager + source |
| No troubleshooting | Support burden | Top 5 errors with fixes |
| Outdated badges | Looks abandoned | Remove or keep current |

---

## AGENTS.md Blurb Template

For CLI tools, include a condensed reference block:

```markdown
## tool — Brief Description

One-line description of what it does and key differentiator.

### Core Workflow

```bash
# 1. Initialize
tool init

# 2. Main operation
tool do-thing

# 3. View results
tool show

Key Flags

--flag1    # Description
--flag2    # Description

Storage

- Location 1: path/to/thing
- Location 2: path/to/other

Notes

- Important caveat 1
- Important caveat 2
```
```

This provides AI agents with scannable reference without loading full README.

---

## Checklist

Before publishing:

```
□ Hero section with illustration + badges + one-liner + curl install
□ TL;DR with Problem/Solution/Feature table
□ Quick example (5-10 commands)
□ At least 3 installation methods documented
□ Every command has usage examples
□ Architecture diagram for complex tools
□ Comparison table vs at least 2 alternatives
□ Troubleshooting section (top 5 errors)
□ Honest Limitations section
□ FAQ with 5+ questions
□ All code blocks are copy-paste ready
□ No broken links or badges
□ Consistent terminology throughout
□ Grammar/spelling checked
```

---

## Badge Reference

Common badges for GitHub READMEs:

```markdown
# CI Status
[![CI](https://github.com/USER/REPO/actions/workflows/ci.yml/badge.svg)](https://github.com/USER/REPO/actions/workflows/ci.yml)

# License
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![License: Apache 2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

# Version/Release
[![GitHub release](https://img.shields.io/github/v/release/USER/REPO)](https://github.com/USER/REPO/releases)

# Downloads
[![Downloads](https://img.shields.io/github/downloads/USER/REPO/total)](https://github.com/USER/REPO/releases)

# Crates.io (Rust)
[![Crates.io](https://img.shields.io/crates/v/CRATE.svg)](https://crates.io/crates/CRATE)

# npm (JavaScript)
[![npm](https://img.shields.io/npm/v/PACKAGE.svg)](https://www.npmjs.com/package/PACKAGE)

# PyPI (Python)
[![PyPI](https://img.shields.io/pypi/v/PACKAGE.svg)](https://pypi.org/project/PACKAGE/)
```

---

## Real-World Examples

Study these READMEs for patterns:

| Project | Notable Pattern |
|---------|-----------------|
| [xf](https://github.com/quangdang46/xf) | Comprehensive CLI docs, search deep-dives |
| [ripgrep](https://github.com/BurntSushi/ripgrep) | Benchmarks, comparison tables |
| [bat](https://github.com/sharkdp/bat) | GIF demos, feature highlights |
| [exa](https://github.com/ogham/exa) | Screenshot galleries, color themes |
| [starship](https://github.com/starship/starship) | Preset configurations, installation matrix |
| [jq](https://github.com/jqlang/jq) | Tutorial progression, manual links |

---

## Advanced: Progressive Disclosure for Long READMEs

For READMEs exceeding 1000 lines, use collapsible sections:

```markdown
<details>
<summary><strong>Advanced Configuration</strong></summary>

Content that most users don't need on first read...

</details>
```

Or link to separate docs:

```markdown
## Documentation

- [Installation Guide](docs/installation.md)
- [Configuration Reference](docs/configuration.md)
- [API Documentation](docs/api.md)
- [Contributing Guide](CONTRIBUTING.md)
```

Keep the README itself focused on the 80% use case.
