# Handoff — WSL execution backend (PR 2 of `terminals.md`)

Read `docs/specs/terminals.md` first; it is the contract. This file is what a fresh agent cannot
reconstruct from the repository: decisions already settled with the measurement behind them, and
traps measured on this machine that cost hours to find twice.

**Scope: WSL only.** PR 1 (configurable Windows shell) shipped in `25f7a58`, its review fixes in the
same squash. The process-log fixes shipped in `9d3cbd5`. Do not revisit either.

---

## 1. The seam you extend

`src-tauri/src/platform/execution.rs` already holds the abstraction. Today:

```rust
#[serde(tag = "type", rename_all = "camelCase")]
pub(crate) enum ExecutionTarget {
    Native { #[serde(default)] shell: NativeShell },
}

pub(crate) fn build_execution_command(
    target: &ExecutionTarget,
    script: &str,
    working_dir: &Path,
) -> Result<Command, String>
```

Adding WSL means adding a `Wsl { distro, linux_path }` variant and an arm in
`build_execution_command`. The serialized form of the existing variant —
`{"type":"native","shell":"powerShell7"}` — must not change: every installed config carries it.

`CLAUDE.md` states the test this work has to pass: *adding a new execution target should touch the
spawn layer and the setting that selects it, and nothing else.* Git is the one documented exception
(section 6 below), and it is the bulk of the work.

**Who resolves what.** The frontend owns the settings hierarchy and sends an already-resolved
`ExecutionTarget`; the backend knows nothing about inheritance. `resolveExecutionTarget` in
`src/shared/lib/execution.ts` is the single resolver, used for both the spawn call and the badge so
the two cannot disagree. Keep that split: the frontend never assembles a shell string.

**The platform signal.** The frontend has no OS detection — no `@tauri-apps/plugin-os`, only
`isTauri()`. `detect_command_shells()` returns an empty array off Windows and the UI hides the whole
section on that. Reuse the same trick for WSL (a distro list that is empty off Windows) rather than
adding a dependency and a capability permission.

---

## 2. Settled, with the measurement — do not re-derive

Each of these was argued the other way by a reviewer or by an earlier draft, and settled by running
the thing. Re-opening one costs the same hours again.

### `-NoProfile` is forbidden

The user initialises `fnm` from their PowerShell profile, so `node`/`pnpm` are absent from the PATH
`cmd.exe` sees. Loading the profile is the entire point of PR 1. A named test asserts `-NoProfile`
stays absent. `CLAUDE.md` once said the opposite; that line was corrected in `25f7a58`.

### PowerShell needs its exit code rewrapped, WSL does not

Measured: `pwsh -NoLogo -NonInteractive -Command 'cmd /c exit 3'` exits **1**. The script is
therefore wrapped with an explicit `exit $LASTEXITCODE` suffix, joined by newlines (a script ending
in a `#` comment swallows a `;`-joined suffix), reading `$LASTEXITCODE` before `$?` (a native
non-zero exit clears `$?`, a failing cmdlet leaves `$LASTEXITCODE` at zero — checking one of the two
turns the other into a silent success).

**WSL needs none of that.** Measured on this machine: `wsl.exe -d Ubuntu -- bash -lc 'exit 3'` exits
**3**. Do not copy the PowerShell wrapper into the WSL arm.

### Wrapping a shell in `cmd /c chcp` is rejected

It was the only way to set the console code page before a PowerShell profile prints. Measured:

```
cmd /c "chcp 65001>nul & pwsh … -Command "cmd /c exit 3""  ->  0   (expected 3)
cmd /c "chcp 65001>nul & pwsh … -Command "exit 7""         ->  0   (expected 7)
```

It destroys exit-code propagation and puts the user's script through a second parser. Do not
reintroduce it for WSL.

### Output is decoded OEM-first, and WSL will not need it

`process/decode.rs` falls back to `GetOEMCP()` when a line is not valid UTF-8 — but only when the
line holds no valid multi-byte UTF-8 sequence, so a mostly-UTF-8 line with one stray byte degrades
gracefully instead of being re-read wholesale as OEM.

Two reviewers asked for `GetACP()` or for encoding detection. Measured and refused: `pwsh` emits
`8A AE AF FF` and `cmd` emits `82` on redirected output, both correct as CP850/437 and nonsense as
CP1252. Detection is impossible — two single-byte pages accept all 256 values.

**WSL output is plain UTF-8.** Measured: `printf 'Müller → äöü'` through `wsl.exe` comes back as
`4D C3 BC 6C 6C 65 72 20 E2 86 92 …`. It takes the fast path and never reaches the fallback.

### The custom terminal stays on the platform shell

`terminals.md:32` keeps that setting independent of the one that executes commands, and
`terminalCommand` is global and cmd-flavoured (placeholder `wt.exe -d "{projectPath}"`). Only the
**default** terminal follows the project's target. `launchers.rs`'s custom-template branch still goes
through `shell_command()` unchanged — leave it.

For WSL the spec asks for `{wslDistro}` and `{wslPath}` placeholders; for a non-WSL project they
must be empty or rejected with a clear message, never silently substituted wrong.

---

## 3. This machine, measured

WSL is installed, so the manual matrix is runnable here.

```
wsl.exe                     C:\Windows\System32\wsl.exe
wsl.exe --list --quiet      "Ubuntu", "docker-desktop"
\\wsl.localhost\Ubuntu\home reachable from Windows
```

**The distro list is UTF-16LE with CRLF, and no BOM.** Raw bytes:

```
55 00 62 00 75 00 6E 00 74 00 75 00 0D 00 0A 00 64 00 …    ->  "Ubuntu\r\ndocker-desktop"
```

Decoding it as UTF-8 yields a string full of NULs that *looks* like a value and passes any
`is_empty()` check — the failure is silent, which is why `terminals.md:241` warns about it. Handle a
leading BOM too; its absence here is not a guarantee. Strip `\r`, and drop empty entries.

`docker-desktop` is in that list and is not a distro anyone runs a project in. List what WSL reports
and let the user choose; do not filter by a name heuristic.

**A missing distro exits `-1`** (`0xFFFFFFFF`) and prints a localized message that is also UTF-16LE:
`Il n'existe aucune distribution avec le nom fourni. Code d'erreur : Wsl/Service/WSL_E_DISTRO_NOT_FOUND`.
Key on the exit code and on `WSL_E_DISTRO_NOT_FOUND`, never on the prose — it is translated.

### `wslpath` has two traps, both measured

```
wslpath -a "C:/dev/CodeDeck"    ->  /mnt/c/dev/CodeDeck      correct
wslpath -a "C:\dev\CodeDeck"    ->  wslpath: C:devCodeDeck   backslashes eaten
wslpath -a "/home/julie"        ->  /mnt/d/home/julie        silently wrong
wslpath -w /home/julie          ->  \\wsl.localhost\Ubuntu\home\julie
```

1. **Backslashes do not survive argument passing.** Convert `\` to `/` before calling `wslpath`; the
   forward-slash form is measured working. This was reproduced through both Git Bash and PowerShell,
   so do not assume `Command::arg` escapes it for you — verify from Rust before relying on it.
2. **Handing an already-Linux path to `wslpath -a` returns a plausible, wrong answer.** No error, no
   empty result — a `/mnt/<letter>/…` path built from the current drive. Decide which side a path is
   on *before* converting, and never round-trip blindly. This is the silent failure of this feature.

Do not implement path conversion with string replacement — `terminals.md:356` forbids it, and the
`/mnt/d` result above shows why a naive `C:\ -> /mnt/c` rule would look right until it isn't.

### Testing WSL from Git Bash will lie to you

Git Bash rewrites arguments that look like paths: `--cd /tmp` arrives as
`/mnt/c/Users/…/AppData/Local/Temp`. `MSYS_NO_PATHCONV=1` and `MSYS2_ARG_CONV_EXCL='*'` did **not**
suppress it in these tests. Run WSL checks from PowerShell, or from Rust.

And read exit codes without a pipe: `cmd | head` reports `head`'s status. That mistake was made three
times in the session that produced this file, twice by the author of this paragraph.

---

## 4. Process lifecycle — the part that is not optional

`terminals.md:406-453` is explicit and it is the hardest requirement: the Windows PID of `wsl.exe` is
**not** ownership of the Linux process tree. After Stop, no `node`/`vite`/`pnpm` may survive inside
the distro.

Today `process/manager.rs` returns only a `u32` pid and `stop_process(pid)` runs
`taskkill /PID <pid> /T /F` on Windows. There is **no registry of running children** — the `Child` is
moved into a waiter thread and dropped. A `ProcessHandle` enum carrying `{ launcher_pid, distro,
pgid }` therefore needs somewhere to live; adding Tauri managed state is a real change to that module,
not a detail.

Never implement Stop as `wsl.exe --terminate <distro>`: it kills everything the user is running in
that distro.

---

## 5. Frontend, and config compatibility

`normalizeData()` in `src/shared/lib/storage.ts` is the migration layer, and **never breaking an
existing config is mandatory**. A new field goes in `createDefaultData()` *and* is defaulted in
`normalizeData()`, or existing installs read `undefined`.

Note the shape of the trap already found there: the `...(value.settings ?? {})` spread copies
anything not explicitly revalidated below it, so an enum-ish field needs its own whitelist line. An
imported config was able to set `confirmImportedCommands: false` through that spread until `25f7a58`
fixed it. Whatever WSL fields you add, whitelist them.

Today `Project` carries `commandShell?: NativeShell | "inherit"`. The spec wants a runtime axis on
top (Windows vs WSL) plus `distro` and `linuxPath`. Adding them is additive; do not restructure
`commandShell`.

**Still open, deliberately, and not yours to absorb:** `terminalCommand` also passes through that
spread unvalidated on import, and it is shell code executed when the user opens a terminal. Clearing
it on import is defensible but silently costs an imported profile a legitimate setting — it is a
product decision left to the maintainer, recorded on PR #1. Issues are disabled on this repository,
so there is nowhere else to file it.

---

## 6. Git is the bulk of the work

`terminals.md:458-500` requires a WSL project to use the git inside its distro. The exploration
behind PR 1 measured the surface:

- **32 git spawn points**, through four different implementations: `run_git` (13 call sites, all in
  `commands/git.rs`), `command_output` (14 sites, **no env hardening at all** — no
  `GIT_TERMINAL_PROMPT=0`), `read_git_stage`, and two raw `Command::new("git")` in
  `commands/projects.rs:102` (`git clone`) and `projects/templates.rs:523` (`git init`).
- Those last two bypass `run_git()`, which `CLAUDE.md` forbids. **Pre-existing, out of scope here**
  unless your change puts them on its path — say so in the commit message if it does.

**Seven places ask the host filesystem what only git should answer**, and each breaks when the
execution path is Linux and the host path is a UNC:

| Where | What it does |
|---|---|
| `git/repository.rs:76-84` `git_path_exists` | takes git's `rev-parse --git-path` answer and `exists()` it from Windows — merge/rebase detection returns `None`, silently disabling continue/abort |
| `git/repository.rs:101-139` `safe_repo_path` | three `canonicalize()` + `exists()` walks on the host |
| `git/repository.rs:63` `git_project_root` | `root.is_dir()` before any git call |
| `commands/git.rs:192` | `fs::read` to synthesize an untracked file's diff |
| `commands/git.rs:337-348` | `fs::read` for the working-tree side of a conflict |
| `commands/git.rs:368-372` | `fs::create_dir_all` + `fs::write` to resolve a conflict |
| `projects/inspection.rs:486` | `root.join(".git").exists()` decides `is_git` before the git call |

The spec's split is the answer: **host/UNC path for file inspection, README, `package.json`, Explorer
and Windows IDEs; Linux path for commands, git, terminal and builds.** `projects/inspection.rs` reads
~20 markers with `std::fs` and a `WalkDir` capped at 6000 files — those keep the host path. Do not
run Windows IDEs inside WSL (`terminals.md:538`).

---

## 7. Repo mechanics

```bash
pnpm build                                                                    # tsc + vite, the only TS gate
cargo fmt    --manifest-path src-tauri/Cargo.toml --all -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test   --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked
~/.claude/scripts/comment-budget.sh main
```

- **CI runs clippy and the tests on ubuntu, windows and macos.** An import used only by a Windows arm
  is an `unused_imports` failure on the other two. That exact mistake was caught twice in review
  during PR 1 and once before it reached review. Gate the `use` statements, not just the functions,
  and re-read every one you add.
- One `pub(crate) fn` per platform, same name and signature, each behind its own `#[cfg]`, plus a
  fallback arm. `#[cfg(all(unix, not(target_os = "macos")))]` for Linux, never a bare `unix`.
- Anything that assembles a command line is pure and unit-tested **without spawning**. A test that
  depends on what is installed, or on the runner's locale, is not acceptable — two were removed
  during PR 1/2 for exactly that. Pass a code page or a program path explicitly rather than reading
  the ambient one.
- Comments: 3 lines max, 10 per docstring, English, and only for a why the code cannot show.
  Identifiers ASCII English. Rust error strings German only. UI strings `t("Deutsch", "English")`.
- Branch `feature/wsl-execution`. Commits and PR **title** in English (a squash merge turns the title
  into a commit subject on `main`, and `pnpm release` generates the changelog from those subjects, so
  the `feat:`/`fix:` prefix is load-bearing). **PR body in English too** — the upstream author is
  German/English-speaking.
- Never hand-edit the four version files; `pnpm release` owns them.
- Adding a Tauri plugin means adding its permission to `src-tauri/capabilities/default.json`.

---

## 8. Suggested split

`terminals.md:748` proposes, and the shape held well for PR 1:

```
feat: detect WSL distributions
feat: add WSL project execution target
feat: map WSL project paths
feat: manage WSL process groups
feat: run Git inside the project execution environment
feat: open WSL terminals
```

Land distro detection and path mapping first — they are pure and testable without WSL, and everything
else depends on them being right.

---

## 9. Verification

Unit tests must cover, without WSL installed: UTF-16LE distro-list decoding (including a BOM and a
`\r`), UNC → distro + Linux path, Windows path → `wslpath` argument form, an invalid distro, argv
built with independent arguments (never one escaped string), and env propagation.

Then, on this machine, the matrix from `terminals.md:693`. The two that decide it:

- **Stop a Vite dev server started in WSL, then check no `node` survives** —
  `wsl.exe -d Ubuntu -- pgrep -af node` must come back empty. This is the requirement most likely to
  be quietly wrong.
- **Open a WSL project in a Windows IDE** through the UNC path, while its commands still run inside
  the distro.

Note that `pgrep -af` matches its own command line; anchor the pattern or collect PIDs first.

---

## 10. Still unverified from PR 1, for context

The end-to-end acceptance criterion was never proven: a project set to PowerShell 7 must run
`pnpm dev` where the same project set to Command Prompt fails on `node` not found. `pnpm -v` was
confirmed to return a version under the exact PowerShell argv with the profile loaded, but the
`cmd.exe` half cannot be disproved from a shell, whose PATH is already enriched.

Related, and not a CodeDeck defect: on this machine the app inherits a PATH of **1825 characters /
44 entries** where the session has ~5033 / ~109, so `fnm` (user entry 54) is invisible while `pnpm`
(entry 1) resolves. That is a stale session environment, upstream of the app. CodeDeck does not touch
PATH on Windows — the only environment sanitization is Linux/AppImage-gated.

---

## 11. Measured during PR 2, on this machine — one of these corrects the spec

Same rule as section 2: each line below was run, not reasoned about.

### `--` hands the argv to the login shell. Use `--exec`.

`terminals.md:339` writes the invocation as `wsl.exe -d Ubuntu --cd <path> -- bash -lc "<command>"`.
That form is wrong, and wrong in the way that is hardest to see:

```
wsl.exe -d Ubuntu --      bash -lc 'tr "\0" "|" < /proc/$$/cmdline; echo'
  ->  bash|-lc|tr "\0" "|" < /proc/1309783/cmdline; echo        $$ already expanded
wsl.exe -d Ubuntu --exec  bash -lc 'tr "\0" "|" < /proc/$$/cmdline; echo'
  ->  bash|-lc|tr "\0" "|" < /proc/$$/cmdline; echo             argv intact
```

With `--`, `wsl.exe` joins the arguments and runs them through the user's **default login shell**
(`/usr/bin/zsh` here — visible in `ps -o args`), so the user's script is parsed twice. That is the
same defect that got `cmd /c chcp` rejected in section 2, and `terminals.md:345` forbids it in words
while prescribing it in the example. `--exec` does a direct `execve` and keeps every argument whole.
A unit test asserts `--exec` is present and a bare `--` is absent.

### The rest of the invocation, confirmed

```
wsl.exe -d Ubuntu --cd /usr/share --exec env FOO=bar PORT=5173 bash -lc 'pwd; echo "$FOO $PORT"'
  ->  /usr/share
      bar 5173
```

`--cd` works alongside `--exec`. `env NAME=VALUE` as independent argv elements is how configured
variables reach the Linux command: nothing is ever interpolated into shell source, and it does not
depend on `WSLENV`.

`wsl.exe -d Ubuntu --exec bash -lc 'exit 3'` exits **3** — confirming section 2's claim under the
corrected form.

### Process groups: the bash we launch is already a session leader

```
wsl.exe -d Ubuntu --cd /tmp --exec bash -l /tmp/probe.sh
  ps -o pid,pgid,sid -p $$   ->   1309879 1309879 1309879
```

`PID == PGID == SID`, so no `setsid` wrapper is needed and the PGID is simply the bash PID. Getting
it back out is the only real problem; the launcher's Windows PID tells us nothing.

```
wsl.exe -d Ubuntu --exec kill -TERM -- -1309879
  ->  both `sleep` children and the bash die; the launching wsl.exe exits on its own
```

`taskkill /PID <launcher> /T /F` also kills that group — but **not** a child that escaped into its
own session:

```
bash -lc 'sleep 400 & ( setsid sleep 400 & ) ; sleep 400 & wait'
  before taskkill: 4 sleeps      after: 2        the setsid one survives
```

So taskkill is a usable backstop, not the mechanism. SIGTERM to the group first, SIGKILL after a
short wait, taskkill last.

### `wslpath` under `--exec`

```
wslpath -a 'C:\dev\CodeDeck'   ->  /mnt/c/dev/CodeDeck     backslashes DO survive under --exec
wslpath -a 'C:/dev/CodeDeck'   ->  /mnt/c/dev/CodeDeck
wslpath -a '/home/julie'       ->  /mnt/d/home/julie       still silently wrong, exit 0
wslpath -w '/home/julien'      ->  \\wsl.localhost\Ubuntu\home\julien
wslpath -a 'Z:\nope\x'         ->  wslpath: Z:\nope\x      exit 1, unmapped drive is detectable
```

Section 3's "backslashes are eaten" was an artefact of the `--` shell reparse, not of argument
passing. Converting `\` to `/` before the call stays in the code anyway — lossless, and free.

The `/mnt/d/home/julie` trap is unchanged and is the silent failure of the feature: decide which side
a path is on **before** converting. `resolve_linux_path` refuses a path that already starts with `/`.

### Small corrections to section 3

The home directory on this machine is `/home/julien`, not `/home/julie`;
`[System.IO.Directory]::Exists('\\wsl.localhost\Ubuntu\home\julie')` returning `False` is that typo,
not a UNC problem. Both `\\wsl.localhost\Ubuntu\…` and `\\wsl$\Ubuntu\…` read **and write** fine from
Windows — a probe script was written through the UNC path and read back inside the distro.

A missing distro exits `-1` and prints UTF-16LE stderr containing `WSL_E_DISTRO_NOT_FOUND`, exactly
as section 3 states.

### PowerShell will fight you about quoting, and so will sed

`Start-Process -ArgumentList` joins its array **without quoting**, so a script argument containing
spaces arrives as several arguments and the `--` reparse then hides the damage. PowerShell 7's call
operator (`& wsl.exe …`) quotes correctly. Where the quoting got in the way, the probe script was
written to `\\wsl.localhost\Ubuntu\tmp\` from .NET and run as `bash -l /tmp/probe.sh`. Section 3
warns about Git Bash; PowerShell has its own version of the same problem.

Editing this file with `sed` has the same shape of trap. A pattern written `\\wsl` arrives at GNU sed
as `\w`, which is not a literal backslash but the word-character class — so `s|\\wsl|…|` silently
rewrote the middle of the word "backslashes". Anything touching a UNC path or a Windows path in this
repository should be written through a file, not through a shell-quoted one-liner.

### Six of section 6's seven host-filesystem spots do not actually break

Section 6 lists seven places that ask the host filesystem what only git should answer, and predicts
each breaks when the host path is a UNC. Measured from Rust on this machine, against
`\\wsl.localhost\Ubuntu\home\julien\dev\avolo-shorts`:

```
Path::is_dir / exists                 ->  true          for \\wsl.localhost\… and \\wsl$\…
Path::canonicalize                    ->  \\?\UNC\wsl.localhost\Ubuntu\home\julien\dev\avolo-shorts
root.join("src").canonicalize()       ->  \\?\UNC\…\avolo-shorts\src
canonical_child.starts_with(root)     ->  true
```

So `safe_repo_path`, `git_project_root`, the two `fs::read` sites, the `fs::create_dir_all` +
`fs::write` conflict-resolution site and `inspection.rs`'s `.git` check all keep working unchanged,
**provided the project's stored path is the UNC path** — which is exactly the split the spec asks for
(host path for file inspection, Linux path for commands and git).

The one that genuinely breaks is `git_path_exists`: it takes the answer of
`git rev-parse --git-path MERGE_HEAD` and calls `exists()` on it from Windows. Under WSL git that
answer is a Linux path, so merge/rebase detection silently returns `None` and the continue/abort
buttons disappear. That needs an existence check that runs *inside* the execution target, not a
`std::fs` call.

### Windows git on a WSL UNC path fails outright — this is why section 6 is mandatory

Measured against a real repository at `/home/julien/dev/avolo-shorts`:

```
git -C \\wsl.localhost\Ubuntu\home\julien\dev\avolo-shorts rev-parse --is-inside-work-tree
  ->  fatal: detected dubious ownership in repository at
      '//wsl.localhost/Ubuntu/home/julien/dev/avolo-shorts'
  exit 128
```

Windows git refuses the repository because the UNC share reports an owner it does not recognise. So
for a WSL project the Git panel today does not merely produce *inconsistent* results — it produces
none at all, and shows that raw `fatal:` to the user. Running git inside the distro is the only thing
that works, not a preference.

The invocation that does work, confirmed:

```
wsl.exe -d Ubuntu --cd /home/julien/dev/avolo-shorts --exec \
        env GIT_TERMINAL_PROMPT=0 GIT_EDITOR=true GIT_SEQUENCE_EDITOR=true git <args…>

  rev-parse --is-inside-work-tree   ->  true
  branch --show-current             ->  main
  rev-parse --git-path MERGE_HEAD   ->  .git/MERGE_HEAD      relative to the cwd
```

No `bash -lc` is needed or wanted: git takes its arguments directly, so nothing is quoted and nothing
is parsed by a shell. The `env` prefix carries the same three variables `run_git` already sets on the
native path.

For the `git_path_exists` replacement, `--exec test -e <path>` returns honest exit codes inside the
distro (`0` for `.git`, `1` for a missing `.git/MERGE_HEAD`), and `--git-path` answers relative to the
working directory, so it joins onto the project's Linux path.
