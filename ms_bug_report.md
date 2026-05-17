# `ms` Bug Report

**Repo tested:** `quangdang46/ms` (release `v0.1.0`, commit on `main` as of 2026-05-16)
**Installer:** `curl -fsSL "https://raw.githubusercontent.com/quangdang46/ms/main/install.sh?$(date +%s)" | bash`
**Host:** Linux x86_64, Ubuntu 22.04, glibc 2.35
**Reporter:** Devin (automated comprehensive feature test)
**Scope note:** Issues specific to the `cass` integration and the `dcg` integration have been **excluded** from this report (CASS is being removed and DCG is being swapped for the upstream `Dicklesworthstone/destructive_command_guard`). What follows is everything else.

I ran the installer, then exercised every top-level subcommand exposed by `ms --help` against a fresh `.ms/` initialized in `/home/ubuntu/ms_test/` with three hand-written `SKILL.md` files and one template-created skill. Every issue below has a copy-paste repro and the exact output captured from the binary built from `main` (semantically identical to the `v0.1.0` release artifact).

The bugs cluster into 5 themes:

1. **Installer is broken on a very common Linux baseline (Ubuntu 22.04 / glibc 2.35).**
2. **Many subcommands print `Error: ...` then exit 0.** This silently breaks any script that does `set -e` or checks `$?`.
3. **`ms show` / `ms load` don't accept their own canonical IDs** (e.g. `local/rust-error-handling`) — the form that `ms route -O json` emits as `skill_id`.
4. **Skills created via `ms template apply` have empty `provider` / `canonical_id` / `display_id`,** which corrupts list/TSV/JSON output and breaks lookups.
5. **README/CLI documentation drift** — several flags shown in the README don't exist (`--all` on `validate` / `fmt`, two-positional `alias add`, `contract show`, `remote add --type`).

---

## 1. Installer (`install.sh`)

### 1.1 [P0] `install.sh` forces `musl` on glibc < 2.38, but no musl artifact is published

**Repro:**

```bash
curl -fsSL "https://raw.githubusercontent.com/quangdang46/ms/main/install.sh?$(date +%s)" | bash
```

**Actual output on Ubuntu 22.04 (glibc 2.35):**

```
[ms] Installing ms...
[ms] Detected platform: x86_64-unknown-linux-musl
[ms] Fetching latest version...
[ms] Installing version: v0.1.0
[ms] Downloading from https://github.com/quangdang46/ms/releases/download/v0.1.0/ms-0.1.0-x86_64-unknown-linux-musl.tar.gz...
curl: (22) The requested URL returned error: 404
[ms] Download failed (attempt 1/3); retrying in 2s
curl: (22) The requested URL returned error: 404
[ms] Download failed (attempt 2/3); retrying in 4s
curl: (22) The requested URL returned error: 404
[ms] Download failed after 3 attempt(s): https://github.com/quangdang46/ms/releases/download/v0.1.0/ms-0.1.0-x86_64-unknown-linux-musl.tar.gz
[ms] Download failed: https://github.com/quangdang46/ms/releases/download/v0.1.0/ms-0.1.0-x86_64-unknown-linux-musl.tar.gz
```

**Why:** `detect_platform()` (lines ~93–115 of `install.sh`) chooses `unknown-linux-musl` when the host glibc is older than 2.38. The actual release contains only:

```
ms-0.1.0-aarch64-apple-darwin.tar.gz
ms-0.1.0-aarch64-unknown-linux-gnu.tar.gz
ms-0.1.0-x86_64-pc-windows-msvc.zip
ms-0.1.0-x86_64-unknown-linux-gnu.tar.gz
SHA256SUMS.txt
```

No `x86_64-unknown-linux-musl` artifact exists, so the installer is **guaranteed to fail on every Ubuntu LTS through 22.04** (which still represents the bulk of WSL / cloud VMs / CI runners). Ubuntu 24.04 happens to ship glibc 2.39 and works.

**Fix:** either (a) add a `*-x86_64-unknown-linux-musl.tar.gz` artifact to the GitHub Actions release workflow, or (b) make the musl preference fall back to the `-gnu` artifact when the musl artifact 404s, or (c) build the `-gnu` artifact on an older runner (e.g. `ubuntu-22.04`) so it runs on glibc ≥ 2.35.

---

### 1.2 [P0] `-gnu` artifact requires GLIBC 2.38 / 2.39 and cannot run on Ubuntu 22.04

After bypassing the musl detection and downloading the gnu artifact directly:

```bash
curl -fsSL -o /tmp/ms.tar.gz \
  https://github.com/quangdang46/ms/releases/download/v0.1.0/ms-0.1.0-x86_64-unknown-linux-gnu.tar.gz
mkdir -p /tmp/ms_extract && tar -xzf /tmp/ms.tar.gz -C /tmp/ms_extract
/tmp/ms_extract/ms --version
```

```
/tmp/ms_extract/ms: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.38' not found (required by /tmp/ms_extract/ms)
/tmp/ms_extract/ms: /lib/x86_64-linux-gnu/libc.so.6: version `GLIBC_2.39' not found (required by /tmp/ms_extract/ms)
```

The comment in `install.sh` even acknowledges this:

> *"On Ubuntu 22.04 LTS (glibc 2.35), the gnu build fails with `GLIBC_2.38 not found`, so musl is the only one that runs."*

…but the musl build is never produced. Combined with §1.1 this means **the README's primary install command does not work on Ubuntu 22.04** (the most common dev/CI distro on Earth right now).

**Workaround used in this test:** `rustup update stable` to 1.95, then `cargo build --release` (about 6 minutes, plus needed to update toolchain because `rust-toolchain.toml` says `channel = "stable"` but the box had 1.83 and the crate requires `rust-version = "1.85"`).

---

### 1.3 [P2] Installer's "Fix" advice on GLIBC mismatch is incorrect

`install.sh` lines ~417-420 print:

```
This is often caused by a GLIBC version mismatch on older Linux
distributions. Try building from source with 'cargo install --path .'
after cloning https://github.com/quangdang46/ms
```

That doesn't fix the underlying problem (the prebuilt binary is still broken) and `cargo install --path .` requires `rust ≥ 1.85`, which Ubuntu 22.04's distro Rust does not satisfy. Suggest: tell users to `rustup update stable` first, or just `cargo install --git https://github.com/quangdang46/ms`.

---

## 2. Exit-code bugs (`Error: ...` printed but `$? == 0`)

This is a systemic problem — at least **8 subcommands / argument errors** print a user-visible error and still return `0`. Any script using `set -e`, `&&`, or `[[ $? == 0 ]]` silently does the wrong thing.

| Command | Output | Actual exit |
|---|---|---|
| `ms show local/rust-error-handling` | `Error: Skill not found: skill not found: local/rust-error-handling` | **0** (should be ≠0) |
| `ms load rust-eh` (alias missing) | `Error: Skill not found: skill not found: rust-eh` | **0** |
| `ms lint` (no path, no `--all`) | `Error: Config error: No SKILL.md found. Specify a path or use --all` | **0** |
| `ms quality` (no skill, no `--all`) | `Error: Config error: missing skill (or use --all)` | **0** |
| `ms install ./bogus.msb` | `Error: Skill validation failed: bundle source not found: ./bogus.msb` | **0** |
| `ms inbox` | `Error: Config error: agent mail is disabled; ...` | **0** |
| `ms list invalid-arg` | `error: unexpected argument 'invalid-arg' found ...` (clap) | **0** (clap normally returns 2) |
| `ms validate --all` | `error: unexpected argument '--all' found ...` | **0** |
| `ms fmt --all` | `error: unexpected argument '--all' found ...` | **0** |
| `ms contract show debug` | `error: unrecognized subcommand 'show' ...` | **0** |

> **Note:** `ms show no-such-skill` (un-prefixed, truly nonexistent) **does** return exit code 1 — so the show command can return non-zero, it just doesn't for canonical-prefixed IDs. The inconsistency is in the lookup path, not the error-reporting path.

These appear to be `anyhow::Result` chains where the caller `println!`s the error and then `Ok(())`s instead of returning the `Err`. Recommend a single `main()` wrapper that maps all `Err` returns to `process::exit(1)` and reserves `exit(2)` for clap-level argument errors.

---

## 3. `ms show` / `ms load` reject their own canonical IDs

**Repro:**

```bash
ms route "rust async runtime panic" -O json
# Returns:
#   "skill_id": "async-tokio",
#   "load_command": "ms load async-tokio --section overview -O json"
# ...but the actual display in `ms list` is `local/async-tokio`.

ms show local/rust-error-handling
# Error: Skill not found: skill not found: local/rust-error-handling
ms show rust-error-handling
# Works.
```

`ms list` and `ms list -O tsv` print the canonical id as `local/rust-error-handling`. The AGENTS.md / README protocol tells the agent to "take the first item in `candidates[]` and run **its `load_command` verbatim**". If `route` ever returned a canonical-prefixed id (as `list` does) the agent would deadlock.

Also note the doubled error text: **`Error: Skill not found: skill not found: ...`** — the outer wrapper and inner error both contain "skill not found".

---

## 4. `ms template apply` produces skills with empty IDs

**Repro:**

```bash
ms template apply debugging --id my-debug --name "MyDebug" \
  --description "Test debug skill" --tag rust,build
ms list -O tsv
```

**Output:**

```
id      name    version layer   quality modified_at     is_deprecated
        MyDebug 0.1.0   project 0.00    2026-05-16 12:34:40     false   <-- empty id field
local/async-tokio       async-tokio       0.1.0   project 0.49 ...
local/python-logging    python-logging    0.1.0   project 0.49 ...
local/rust-error-handling rust-error-handling 0.1.0 project 0.49 ...
```

`ms show my-debug --meta`:

```json
{
  "id": "my-debug",
  "provider": "",
  "canonical_id": "",
  "display_id": "",
  ...
}
```

Compare to an indexed skill:

```json
{
  "id": "rust-error-handling",
  "provider": "local",
  "canonical_id": "local/rust-error-handling",
  "display_id": "rust-error-handling",
  ...
}
```

Template-created skills are written directly under `.ms/archive/skills/by-id/<id>/SKILL.md` and never get a `provider` assigned. They:

- Don't show up properly in `ms list -O tsv` (empty first column).
- Get `quality_score: 0` in JSON output where indexed skills get the default `0.485`.
- Inflate `ms graph keystones` (the empty-id skill shows up with score `1.0000`, see §6).

**Fix:** `template apply` should set `provider = "local"` and recompute `canonical_id` / `display_id` like the indexer does, then add to the search index.

---

## 5. README / CLI documentation drift

Cases where the README documents a flag or invocation that the actual CLI does not support:

### 5.1 `ms validate --all` and `ms fmt --all`

Both `lint` and `quality` print `(or use --all)` in their error messages, but `validate` and `fmt` don't accept `--all` even though the same UX would be expected. README does not show them being used with `--all`; the inconsistency is between subcommands.

### 5.2 `ms alias add <alias> <target>`

README example:

```
ms alias add rust-eh rust-error-handling
```

Actual help (`ms alias add --help`):

```
Usage: ms alias add [OPTIONS] --target <TARGET> <ALIAS>
```

So the README's positional-`<target>` invocation fails with `error: unexpected argument 'rust-error-handling' found`. Either change the README to `ms alias add rust-eh --target rust-error-handling` or accept positional target.

### 5.3 `ms contract show <id>` does not exist

README's `Pack Contracts` section mentions inspection of built-in vs custom contracts via `ms contract list`. There is no `ms contract show` subcommand even though `ms template show` exists — `ms contract show debug` returns:

```
error: unrecognized subcommand 'show'
Usage: ms contract [OPTIONS] <COMMAND>
```

### 5.4 `ms config set` / `get` (read/write subcommands)

README uses `ms config <key> <value>`. The help also lists `--list` and `--unset`. There is no `set`/`get` subcommand (`ms config get foo` works as a no-op with exit 0 and produces no output — see §7.1).

### 5.5 `ms remote add` `--type` vs `--remote-type`

README:

```
ms remote add origin /path/to/archive --type filesystem
```

Actual help (`ms remote add --help`):

```
--remote-type <REMOTE_TYPE>   Remote type (filesystem|git|ru|jfp-cloud)
```

The flag is `--remote-type`, not `--type`.

### 5.6 Hand-written skills must be named `SKILL.md`

The README "Hand-Written SKILL.md Files" section shows a single markdown file. In practice, `ms index` only picks up files named **exactly** `SKILL.md`:

```bash
$ ls skills/                    # rust-error-handling.md  async-tokio.md  python-logging.md
$ ms index
Indexing skills...

No SKILL.md files found
```

After restructuring to `skills/<name>/SKILL.md`, indexing works. The README should make explicit that the convention is `<dir>/SKILL.md`, not "any .md file under skills/".

### 5.7 The `--robot` global flag

README documents `--robot` as a top-level global flag:

```
--robot     # JSON output to stdout (for automation)
```

`ms --help` does not list `--robot` under Options — only `-O/--output-format`, `-m/--machine`, `--plain`, `--color`, `--theme`, `-v`, `-q`, `--config`. The flag *does* work (`ms --robot list` produces JSON), but it's undocumented in the actual help output, which forces agents reading `--help` to use `-m` or `-O json`.

---

## 6. Graph / search / output formatting bugs

### 6.1 `ms graph export --format mermaid` produces an empty graph

```
$ ms graph export --format mermaid
graph TD;
```

Even though `--format json` exports four nodes, mermaid output has zero. With four skills indexed (and no edges yet), the mermaid output should at least emit `graph TD;\n  my-debug;\n  async-tokio;\n  rust-error-handling;\n  python-logging;`.

### 6.2 `ms experiment list` prints Rust `Debug` format

```
$ ms experiment list
...
Variants    Array [Object {"id": String("control"), "name": String("control"), "weight": Number(0.5)}, ...]
```

The same data is emitted as proper JSON by `experiment create`'s human output:

```
Variants    [{"id":"control","name":"control","weight":0.5}, ...]
```

The `list` path is using `{:?}` on a `serde_json::Value` instead of `to_string`.

### 6.3 `ms graph keystones` includes empty-id template skills with rank 1.0000

```
Keystones (showing 4):
Rank      Score Skill ID                             Name
----      ----- --------                             ----
   1     1.0000 my-debug                             MyDebug          <-- empty canonical_id
   2     1.0000 async-tokio                          async-tokio
   3     1.0000 rust-error-handling                  rust-error-handling
   4     1.0000 python-logging                       python-logging
```

With zero edges in the graph, every node has the same PageRank, so technically "correct", but the template-created skill participating without a provider prefix is a symptom of §4.

### 6.4 `ms suggest` and `ms route` ignore `local/` prefixes

`ms suggest -O json` returns `"skill_id": "rust-error-handling"` (no `local/`), but `ms list -O json` returns `"id": "rust-error-handling"` and `ms list -O tsv` returns `local/rust-error-handling`. There is no single canonical surface form — pick one (preferably the route-emitted unprefixed `display_id`) and use it everywhere.

### 6.5 `compress` `compression_ratio` is the *kept* ratio, not the savings

```
$ ms compress rust-error-handling -O json
"summary_words": 61,
"original_words": 74,
"compression_ratio": 0.8243243243243243,
```

`0.824 ≈ 61/74` is the **fraction retained**, while the human output reports `Compression: 17.6%` (= 1 - 0.824 = "savings"). The two views disagree on the sign of "compression". Pick one definition and use it consistently — most tools report "compression ratio" as `output / input` (current 0.824) so the human output's "17.6%" should be labeled "Savings" or "Reduction", not "Compression".

---

## 7. Silent / no-op commands

### 7.1 `ms config get <key>` prints nothing

```
$ ms config get search.use_embeddings
$ echo "EXIT=$?"
EXIT=0
```

Empty output, no value. The bare positional form `ms config search.use_embeddings` correctly prints `false`/`true`. There is no `get` subcommand, but invoking `get` silently swallows the call instead of erroring.

### 7.2 `ms config <bogus.path> <value>` silently no-ops

```
$ ms config bogus.path 1
$ echo "EXIT=$?"
EXIT=0
$ ms config | grep bogus
(no match)
```

Unknown configuration keys are accepted with exit 0 and not persisted. Should at minimum warn ("unknown configuration key") and return non-zero.

### 7.3 `ms config <key> <value>` (set) is silent

Successful sets produce no confirmation, no diff, nothing on stdout or stderr. A short `set search.use_embeddings = false` line (or `--quiet` to suppress) would help operators verify their write took effect.

### 7.4 `ms fmt <skill>` is silent

```
$ ms fmt rust-error-handling
$ echo "EXIT=$?"
EXIT=0
```

No "formatted N files" message, no "no changes needed". Indistinguishable from a no-op crash.

### 7.5 `ms init -O json` does not honor `-O json` when already initialized

```
$ ms -O json init
! Already initialized at /home/ubuntu/ms_test/.ms
  Use --force to reinitialize
```

When asked for JSON, return JSON (e.g. `{"status": "error", "code": "already_initialized", "path": "..."}`) so robot consumers can parse it.

---

## 8. Other observations

### 8.1 Compile warning in `import/provider.rs`

Building from `main`:

```
warning: unused variable: `e`
   --> src/import/provider.rs:200:21
    |
200 |                 Err(e) => {
    |                     ^ help: if this is intentional, prefix it with an underscore: `_e`
```

Cosmetic only — fix by prefixing with `_` or actually logging the error.

### 8.2 `ms providers sync` lists provider paths that don't exist

```
Provider: /home/ubuntu/.local/share/ms/skills
============================================================
  0 new, 0 changed, 0 unchanged, 0 errors

Provider: ./skills
============================================================
  0 new, 0 changed, 0 unchanged, 0 errors

Provider: .ms/skills
============================================================
  0 new, 0 changed, 0 unchanged, 0 errors

Provider: /home/ubuntu/.local/share/ms/community
============================================================
  0 new, 0 changed, 0 unchanged, 0 errors
```

Two of these (`~/.local/share/ms/skills` and `~/.local/share/ms/community`) don't exist on a fresh box. Sync silently treats nonexistent dirs as "0 unchanged" — would be more helpful to print `(missing)` or `(no such directory)` so the user can decide whether to create them.

### 8.3 `ms browse` requires a TTY but fails loudly

```
$ ms browse </dev/null
Error: Skill validation failed: browse command requires an interactive terminal
EXIT=1
```

This one *does* exit 1, but the error type is "Skill validation failed" which is misleading — there's no skill being validated; the issue is the missing TTY. Suggest a dedicated `TerminalRequiredError` variant.

### 8.4 Inconsistent global flags between subcommands

`ms config` accepts `--list` and `--unset`. `ms list` accepts `--tags`, `--layer`, `--limit`, `--offset`. There's no easy way to discover the full surface without running `--help` on every subcommand. Recommend adding a top-level `ms commands` or `ms help --all` that prints a tree of every subcommand with its options (similar to `cargo --list`).

### 8.5 `ms machine info` exposes a UUID; `--robot` would help here

For multi-machine workflows this UUID is useful, but `ms machine info` only has a human view. `ms machine info -O json` would emit the structured form, which is fine — just calling out that it isn't documented in the README.

### 8.6 `ms list -O jsonl` and `ms list -O plain` emit empty output

```
$ ms list -O jsonl
$ echo "EXIT=$?"
EXIT=0
$ ms list -O plain
$ echo "EXIT=$?"
EXIT=0
```

With 3 skills indexed, both `jsonl` and `plain` output nothing on stdout. The JSON view shows them just fine, and the TSV view shows them. This is the **only** time I saw `jsonl` produce empty output — for `search` and `show` it works correctly.

---

## Quick severity table

| # | Issue | Severity | Effort |
|---|---|---|---|
| 1.1 | musl artifact missing → installer 404 on glibc < 2.38 | **P0** | Small (add release artifact) |
| 1.2 | gnu artifact needs GLIBC 2.38/2.39 → fails on Ubuntu 22.04 | **P0** | Medium (build on older runner) |
| 1.3 | Installer's "Fix" advice is incorrect | **P2** | Trivial (docs) |
| 2 | At least 8 commands print error + exit 0 | **P1** | Small (centralize Result handling in `main`) |
| 3 | `show local/<id>` not found | **P1** | Small (id resolution should accept canonical form) |
| 4 | `template apply` skills have empty canonical_id/provider | **P1** | Medium |
| 5.1-5.7 | README documents non-existent flags | **P2** | Small (docs or CLI alignment) |
| 6.1 | `graph export mermaid` empty | **P2** | Small |
| 6.2 | `experiment list` shows Rust Debug format | **P2** | Trivial |
| 6.4 | Surface-form inconsistency (`local/` prefix) | **P2** | Small |
| 6.5 | Compression ratio sign confusion | **P3** | Trivial |
| 7.1-7.5 | Silent no-ops in `config get`/set/fmt/init | **P2** | Small |
| 8.1 | Unused-variable warning | **P3** | Trivial |
| 8.3 | `browse` misclassifies TTY error | **P3** | Trivial |
| 8.6 | `list -O jsonl/plain` empty | **P2** | Small |

---

## How to reproduce all of the above end-to-end

```bash
# 1. Bypass the installer (see §1) and build from source:
git clone https://github.com/quangdang46/ms.git /tmp/ms-src
cd /tmp/ms-src
rustup update stable      # need >= 1.85
cargo build --release
install -m 0755 target/release/ms ~/.local/bin/ms
export PATH="$HOME/.local/bin:$PATH"

# 2. Initialize a clean workspace:
mkdir -p /tmp/ms_test && cd /tmp/ms_test
ms init

# 3. Create three real skill directories:
mkdir -p skills/rust-error-handling skills/async-tokio skills/python-logging
# (write SKILL.md files like in this repo's README — note the dir/SKILL.md
# layout, see §5.6.)

ms index
ms list -O tsv               # → see §4 (empty id) once template skill is added

# 4. Exit-code repros (each prints Error: ... but $? == 0):
ms show local/rust-error-handling   ; echo "EXIT=$?"   # §3
ms quality                          ; echo "EXIT=$?"   # §2
ms install ./bogus.msb              ; echo "EXIT=$?"   # §2
ms inbox                            ; echo "EXIT=$?"   # §2
ms validate --all                   ; echo "EXIT=$?"   # §2 + §5.1
ms fmt --all                        ; echo "EXIT=$?"   # §2 + §5.1
ms list invalid-arg                 ; echo "EXIT=$?"   # §2 (clap)

# 5. Documentation drift repros:
ms alias add rust-eh rust-error-handling             # §5.2
ms contract show debug                               # §5.3
ms config get search.use_embeddings                  # §5.4 + §7.1
ms remote add origin /tmp/x --type filesystem        # §5.5

# 6. Empty-id template skill (§4):
ms template apply debugging --id my-debug --name MyDebug \
  --description "Test" --tag rust
ms show my-debug --meta                              # provider/canonical_id are ""
ms list -O tsv | head -2                             # first row has empty id column

# 7. Graph mermaid empty (§6.1):
ms graph export --format mermaid

# 8. Output-format inconsistency (§8.6):
ms list -O jsonl                                     # empty
ms list -O plain                                     # empty
ms list -O json                                      # works
ms list -O tsv                                       # works
```

Raw command output for each of these is captured under `/home/ubuntu/ms_test/logs/*.log` if it helps with diffing.
