# CLAUDE.md

Guidance for Claude Code (claude.ai/code) working in this repository.

## What this is

`factorio-oracle` runs a real Factorio install headless and reports what it did.
It owns discovery, mod scaffolding, launching and reading results back. It owns
**none** of the analysis.

That split is the whole design. A probe compares the game against a consumer's
own reimplementation, so the comparison has to run in the consumer's language.
The interface is therefore JSON in and JSON out, not a probe framework. Four
repos rely on it: FactorioTools, factorio-blueprint-editor, FactorioMapWebUI and
FactorioWikiDamageThresholds.

**The migration rule is new probes only.** Nothing already working in those
repos gets rewritten to use this. Adoption happens when someone writes a probe
they did not have before.

## Prerequisites

- Rust **1.97.1**, pinned in `rust-toolchain.toml`. The pin exists so a shared
  tool four repos depend on does not change behaviour because a contributor has
  a different rustup default.
- A real Factorio install for the integration tests. Without one they skip
  rather than fail, so a green run on a machine with no game proves less than it
  looks. Check which happened before trusting it.

## Common commands

```bash
# Exactly what CI runs, in order. Run all three before claiming anything passes.
cargo fmt --all -- --check && cargo clippy --all-targets -- -D warnings && cargo test --all-targets

# Find installs
cargo run -- installs list

# Run a probe and read the result
cargo run -- run --probe examples/pumpjack-terminals/probe.json --work-dir /tmp/pj
cat /tmp/pj/write/script-output/oracle-dump.json

# Reproduce FactorioTools' committed fixture
cargo run -- run  --probe dump-data.json --work-dir /tmp/w > /tmp/run.json
cargo run -- trim --run /tmp/run.json --spec trim-spec.json --out fixture.json [--check]
```

Test counts to expect: **159 unit tests**, plus **8 integration tests** split
across three files. `tests/acceptance.rs` has 3: two run offline against a
committed fixture, and one (`the_real_install_reproduces_it_too`) is
install-gated. `tests/provenance.rs` has 2: one always-on, and one gated on
the `FACTORIO_ORACLE_PROVENANCE_DIR` environment variable naming another
repo's fixture directory - not an install gate, so it skips even when
Factorio is present. `tests/real_game.rs` has 3, all install-gated. That is
**4 install-gated tests** in total. Without a real Factorio install they skip
rather than fail, so a green run on a machine with no game proves less than it
looks. Check which happened before trusting it.

## Layout

| Module | Job |
| --- | --- |
| `install.rs` | Finding installs and working out where their pieces live |
| `version.rs` | Reading a version out of `factorio --version` |
| `probe.rs` | The JSON document a consumer hands in |
| `scaffold.rs` | Writing the throwaway mod's files and the isolated config |
| `lua.rs` | The only place this tool writes Lua on a consumer's behalf |
| `args.rs` | The argument vector, which differs per mode |
| `spawn.rs` | The one boundary that touches processes, behind a trait |
| `outcome.rs` | Deciding whether a run succeeded, per mode |
| `run.rs` | Wiring the pure builders to disk and a spawner |
| `numbers.rs` | Preserving the bits the game produced |
| `trim/` | Cutting a full `data.raw` dump down to a consumer's slice |
| `provenance/` | Which Factorio each fixture came from, and whether that record is still honest |

Five run modes, and **the success predicate differs per mode**: `dump-data`
(no mod at all, the mod dir exists only to be empty), `create`, `interactive`
(a human at the keyboard), `preview` (**exits 0** on success), and `read-only`
(files on disk, no binary).

## Measured facts

**Read this section before changing anything that talks to the game.** Every
item cost a failed run to learn, and none is visible from the code. Three real
defects once survived 60 unit tests here and were caught by the first run
against the actual game, because the fake encoded the same wrong beliefs the
code did. **A fake can only be wrong in the ways its author already considered.**

- **Factorio writes nothing to stderr.** Everything, including an
  `error("DUMPED-OK")` sentinel, goes to stdout. Measured three ways on 2.1.14,
  and again on Windows: stderr was zero bytes every time. The original code
  checked stderr, so the sentinel was never seen on any real run.
- **Omission in `mod-list.json` means enabled.** Factorio rewrites the file at
  startup and adds back every bundled mod the file does not mention, with
  `enabled: true`. A file naming only `base` comes back naming five, all loaded.
  An explicit `enabled: false` **is** honoured, so naming a mod is the only way
  to get a smaller game than the install ships with. An empty mod directory
  keeps out *user* mods and nothing else.
- **`read-data` is layout-dependent, so never write it as a relative token.**
  `__PATH__executable__/../data` is right for a macOS bundle and wrong
  everywhere else: Windows and Linux put the binary at `bin/x64/`, so it
  resolves to `bin/data` and the game exits 1 with "There is no package core
  in". Use the absolute `data_dir` that `resolve_layout` already computes.
  Windows accepts either slash style, so `Path::display()` needs no rewriting.
- **`--write-data` is not a CLI flag on any platform.** It is a `config.ini`
  `[path]` key reached with `-c`. `--dump-data` honours it, and the dump lands
  in that directory's own `script-output`, which is what makes a stale dump from
  an earlier capture impossible to pick up.
- **`script.on_init` takes exactly one handler.** A prelude registering it is
  silently replaced by the consumer's own - no error, the handler simply never
  runs. 17 of 18 probes in factorio-blueprint-editor register an `on_init`, so a
  prelude must not. `helpers.write_file` works at `control.lua` toplevel with no
  event at all, which has no collision surface and does not wait a tick.
- **`serde_json` mis-parses long decimal literals by one ULP**, in both
  directions, on values like
  `0.394500000000000028421709430404007434844970703125`. The crate uses
  `arbitrary_precision` and re-parses through `std`'s `f64::from_str`, which is
  correctly rounded and agrees with CPython bit for bit. **Do not remove that.**
- **`preserve_order` must stay off** so `serde_json::Map` is a `BTreeMap` and
  output sorts to match Python's `sort_keys=True`. Byte-for-byte determinism is
  a hard requirement, because `--check` is a diff.
- **`runtime-api.json` publishes no `defines` values.** Across all 1,554 entries
  the only keys are `name`, `order` and `description`. Reading `order` as the
  value is right today only because Factorio declares directions clockwise from
  `north = 0` with no gaps. Read the real values with a probe instead
  (`defines_from: "probe"`).
- **`--dump-data` is byte-identical across architectures.** Same build, macOS
  arm64 Steam and Windows x64 standalone both produce sha256 `acc944e6...` from
  byte-identical inputs. So the acceptance test is not platform-specific.
- **`--create` needs no map-gen settings file**, and `--map-gen-seed` overrides
  a seed inside the settings file. The CLI takes one `seed` and writes both
  channels, which makes the precedence irrelevant.
- **`loadedMods` cannot come from the active-mods prelude**, because
  `script.active_mods` never reports `core` and the fixtures list it. Grep the
  game's stdout for `Loading mod <name>`.
- **A real provenance manifest has two keys per entry, not eight.** Measured
  2026-08-17 across FactorioMapWebUI's 100 entries: every one carries
  `factorioVersion` and `evidence`, and `factorioBuild`, `branch`,
  `loadedMods`, `capturedOn`, `capturedBy` and `targetVersionRange` appear zero
  times. The design sketched all eight. A checker requiring the sketch would
  reject the only real manifest there is, so the other six are optional and
  carried through untouched.
- **`evidence` is free text, and enforcing a grade would reject 48 of 100.**
  First word across the same 100 entries: `stated` 48, `captured` 34,
  `RE-CAPTURED` 8, `inferred` 4, `re-captured` 3, `UNDOCUMENTED` 1, and twice
  it is just `the`. The design's enum was `stated | inferred | unknown`, which
  accepts 52 of the 100 first words and rejects the other 48. The grade that
  is real is `factorioVersion: "unknown"`, which is a field, and that is what
  the ratchet counts.
- **An extension allowlist is how ground truth goes unrecorded.** MapWebUI's
  provenance test globs `.json` and `.png`. Its fixture directory also holds 10
  `.txt` map exchange strings, and **8 of the 10 are read as ground truth** by
  `decode.spec.ts`, `encode.spec.ts` and `jsonExport.spec.ts`. None has an
  entry and none can get one while the glob decides. So this tool names every
  file, and a deliberate non-fixture goes in `notFixtures` with a reason.
- **Manifest keys are relative paths, not bare filenames.** The design says
  bare filenames, which is true of MapWebUI's flat directory and was never
  tested against a tree. This crate's own `tests/fixtures/` is two levels deep.
  The walk joins components with `/` by hand rather than using
  `Path::display()`, so a Windows run and a macOS run produce the same
  committed key.
- **The walk skips dotfiles and dot-directories, plus two Windows names, and
  that is load-bearing on both platforms.** `.DS_Store` appears in any
  directory a Finder window has opened; demanding an entry for it would make
  the check fail on the machine that can fix it and pass in CI. The skip
  `continue`s before the directory branch, so a dot-prefixed name is skipped
  whole even when it is a directory - a consumer keeping fixtures under
  `.golden/` gets nothing recorded for that subtree, not an error. `Thumbs.db`
  and `desktop.ini` (matched case-insensitively) are skipped for the same
  reason on Windows: Explorer writes `Thumbs.db` into any folder of images it
  has thumbnailed, and MapWebUI's fixture directory holds `.png` files.
- **`#[serde(flatten)]` works under `arbitrary_precision`.** Measured
  2026-08-17: a struct with two named string fields plus a flattened
  `BTreeMap<String, Value>` parsed a document holding both an integer and a
  decimal, with no error. Recorded as a negative result so nobody spends an
  afternoon ruling it out. Provenance entries stay a `Value` anyway, because
  only two keys are required and the rest must round-trip untouched.

### Writing a probe

Three documents in `docs/` carry the accumulated probe-writing knowledge, lifted
with attribution from the sibling repos. Read them before writing a new probe;
they are the reason a fifth repo does not have to learn the same lessons again.

- `docs/order-of-attack.md` - factorio-data first, then the oracle, then the
  binary.
- `docs/method.md` - the epistemics. A control must be able to fail while the
  hypothesis holds; last man standing is not a measurement.
- `docs/gotchas.md` - the facts, each of which cost a run. Overlaps this section
  on purpose: the facts above are the ones this tool's own code depends on.

### Writing Lua for a probe

- **Factorio 2.1 deleted `LuaEntity.fluidbox` and the whole `LuaFluidBox`
  class**, flattening it onto `LuaEntity` as `get_fluid_box_*`. So
  `entity.fluidbox.get_pipe_connections(i)` became
  `entity.get_fluid_box_pipe_connections(i)`.
  `entity.prototype.fluidbox_prototypes[i].pipe_connections` still works in
  both, so prototype-level reads need no branching.
- **Feature detection needs a `pcall`.** Reading an unknown key on `LuaEntity`
  *raises* in 2.0 rather than returning nil, so the obvious guard throws on the
  older version:

  ```lua
  -- throws on 2.0
  if entity.get_fluid_box_pipe_connections then ... end
  -- works on both
  local has_new = pcall(function() return entity.get_fluid_box_pipe_connections end)
  ```

- **Run a probe against two versions when you can.** The older one reproduces
  the known-good answer, which is what earns trust in the new one. See
  `examples/pumpjack-terminals`.

### Running the game from a shell

- **`factorio.exe` is a GUI-subsystem binary on Windows.** Launched from
  PowerShell it returns immediately, does not wait, and captures no stdout - the
  run looks like a 0.03 second success that produced nothing. Use
  `Start-Process -Wait -NoNewWindow -PassThru -RedirectStandardOutput`. Rust's
  `Command::spawn` plus `wait_with_output` waits correctly, so the tool itself
  is fine; this bites shell invocations.
- **Nothing here has a timeout by default in the consumer repos**, which is one
  of the things this tool exists to fix. A hung game hangs the capture forever.

## Sources

Two kinds, and the difference decides how each may be used. `README.md` has the
full version.

**Authoritative** sources say what the game accepts, ship with the install, read
offline byte-identically, and are the only things a capture may depend on:
`--dump-data`, `data/*/migrations/*.json`, `doc-html/runtime-api.json`.

**Reference** sources say *why* something changed, which is what tells you which
captured value now needs review. Nothing automated reads them and nothing
should: `data/changelog.txt`, and the Friday Facts blog.

Search the blog at `https://factorio.com/blog/search/<term>`. It covers the
whole archive. Word choice matters: `mirroring` does not return FFF #442, the
post that introduced entity mirroring, while `flip` and `fluid` both do.

## Conventions

- **Hyphens only.** No em dashes or en dashes, in files or commit messages.
- **Comments carry the reasoning, not the mechanics.** The measured facts above
  live beside the code that depends on them, with the measurement and its date.
  That is deliberate: a reader who does not know why `preserve_order` is off
  will turn it on.
- **Prefer measuring the game over reasoning about it.** If a claim about
  Factorio is not backed by a run, say so.
- Do not push or commit unless asked. Branch first if on `main`.
