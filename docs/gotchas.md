# Gotchas

Facts about Factorio and its API that each cost a failed run. Lifted with
attribution from `factorio-blueprint-editor`'s `tools/oracle/README.md` and
`FactorioMapWebUI`'s `test/oracle/README.md`, plus what this tool measured while
being built.

For transferable reasoning rules rather than facts, see [method.md](method.md).
For which source to ask first, see [order-of-attack.md](order-of-attack.md).

Anything here marked **measured** names the version and date. Anything not marked
came across from a sibling repo with its own attribution, and has not been
re-measured here.

## Writing the mod

- **`helpers.write_file` and `helpers.table_to_json`, not `game.*`.** They moved
  in 2.1 and the old names are gone. (factorio-blueprint-editor)

- **`factorio_version` in `info.json` must match the binary's major.minor**, or
  the mod is silently skipped, no dump appears, and nothing in Factorio's output
  names the cause. Derive it from `factorio --version`; never hardcode it. Every
  factorio-blueprint-editor probe up to #141 hardcodes `"2.1"`, which was correct
  only because the only install on that machine was 2.1.12. This tool derives it
  in `version.rs`.

- **Embed a blueprint string or a noise expression in a Lua long bracket**
  (`[==[ ... ]==]`) so base64, braces, quotes and `var('...')` survive verbatim.
  (both repos)

- **`script.on_init` takes exactly one handler.** A second registration silently
  replaces the first, with no error. **Measured 2026-08-16 on 2.1.14.** 17 of 18
  factorio-blueprint-editor probes register an `on_init`, so anything a runner
  injects must not use one.

- **`helpers.write_file` works at `control.lua` toplevel**, with no event at all.
  **Measured 2026-08-16 on 2.1.14.** `script.active_mods` is populated there too.
  That is where metadata belongs: no collision surface, and it costs no ticks.

- **Toplevel is for metadata, not for sampling.** `game.surfaces[1]` does not
  exist at control-stage toplevel, so anything calling `calculate_tile_properties`
  or `get_tile` still needs `on_init`. The two rules compose rather than compete.
  Do not read "toplevel works" as "probes should stop using `on_init`".

- **`--instrument-mod` is worse than useless for a probe runner.** **Measured
  2026-08-16 on 2.1.14.** It does give earlier hooks: `instrument-data.lua` ran
  at 0.045s against `data.lua` at 0.129s. But `instrument-control.lua`'s
  `script.on_init` handler **never fired**, because `control.lua` registered one
  too and the later registration replaced it. So it hands a probe an earlier hook
  whose event registration the consumer then destroys, silently. It would appear
  to work on every probe that does not register `on_init`, and fail on every
  probe that does.

## Launching the game

- **`error("DUMPED-OK")` makes Factorio exit non-zero. That is success.** Key off
  the dump file existing, never the exit code. (both repos)

- **Factorio writes nothing to stderr.** **Measured 2026-08-17, three ways on
  2.1.14 and again on Windows: stderr was zero bytes every time.** A control-stage
  `error()`, a data-stage error, and an unknown command line flag all print to
  stdout. This tool's first version checked stderr for the sentinel, so
  `sentinelSeen` was false on every real run while sixty unit tests passed.

- **Omission in `mod-list.json` means enabled.** **Measured 2026-08-17 on
  2.1.14.** Factorio rewrites the file at startup and adds back every bundled mod
  the file does not mention, with `enabled: true`. A file naming only `base` came
  back naming five, all loaded. An explicit `enabled: false` **is** honoured, so
  naming a mod is the only way to get a smaller game than the install ships with.
  An empty mod directory keeps out *user* mods and nothing else.

- **`read-data` is layout-dependent, so never write it as a relative token.**
  `__PATH__executable__/../data` is right for a macOS `.app` bundle and wrong
  everywhere else. Windows and Linux put the binary at `bin/x64/`, so it resolves
  to `bin/data` and the game exits 1 with "There is no package core in". Use an
  absolute path. Windows accepts either slash style. (this repo, PR #2)

- **`--write-data` is not a CLI flag on any platform.** It is a `config.ini`
  `[path]` key reached with `-c`. `--dump-data` honours it, and the dump lands in
  that directory's own `script-output`, which is what makes a stale dump from an
  earlier capture impossible to pick up. **Measured 2026-08-17 on 2.1.14.**

- **`--create` needs no map-gen settings file.** **Measured 2026-08-16 on
  2.1.14.** It generates a map, loads the mod and dumps with no settings file at
  all. `--map-gen-seed` overrides a seed inside the settings file, so writing one
  `seed` through both channels makes the precedence irrelevant.

- **A blueprint import needs no `--map-gen-settings`** either, since nothing is
  being generated. A noise probe does need one, to route
  `property_expression_names.elevation` at the probe expression. (both repos)

- **`factorio.exe` is a GUI-subsystem binary on Windows.** **Measured 2026-08-17**
  on a standalone win64 install. Launched from PowerShell it returns immediately,
  does not wait, and captures no stdout. The run looks like a 0.03 second success
  that produced nothing. Use
  `Start-Process -Wait -NoNewWindow -PassThru -RedirectStandardOutput`. Rust's
  `Command::spawn` plus `wait_with_output` waits correctly, so this bites shell
  invocations rather than this tool.

- **Nothing in the consumer repos has a timeout**, which is one of the things
  this tool exists to fix. A hung game hangs the capture forever.

## Reading the answer back

- **`--dump-data` is byte-identical across CPU architectures.** **Measured
  2026-08-17 on 2.1.14, build 87180.** macOS arm64 Steam and Windows x64
  standalone, same build, both produced 29,506,804 bytes with sha256
  `acc944e6...` from byte-identical inputs.
  All 3,506 `.lua`/`.json`/`.cfg` files under `data/` matched too. So a capture
  is not platform-specific.

- **`runtime-api.json` publishes no `defines` values.** **Measured 2026-08-17 on
  2.1.14:** across all 1,554 entries the only keys are `name`, `order` and
  `description`.
  Reading `order` as the value happens to be right for directions today, only
  because Factorio declares them clockwise from `north = 0` with no gaps. `order`
  is a dense `0..n-1` index, so it cannot express a gap, a duplicate, or a
  non-zero start. Read real values with a probe.

- **`calculate_tile_properties(property_names, positions)` takes property names
  first.** The HTML docs and `runtime-api.json` list `positions` first and are
  wrong; the `order` field is authoritative. (FactorioMapWebUI)

- **`help()` is gone in 2.1.** `LuaEntity` and `LuaTrain` have no such key, so
  the usual way of asking an object what it can do does not work. The install
  ships `doc-html/runtime-api.json`, the whole runtime API as data, which is a
  better source than either `help()` or `strings` on the binary.
  (factorio-blueprint-editor #115)

- **`MapPosition` is 1/256 fixed-point.** A world offset below `1/256` rounds to
  zero. Choose probe offsets accordingly. (FactorioMapWebUI)

- **Entities snap, so a requested position is not a measured one.** A rail asked
  for at (-4,-4) lands at (-3,-3) on the 2-tile rail grid, and several accepted
  (position, direction) triples collapse to one real placement. Read offsets off
  the created entity: 16 raw acceptances around a straight rail are 4 actual
  signal positions. (factorio-blueprint-editor #133)

- **`loadedMods` cannot come from `script.active_mods`** (this repo), because it
  never reports `core` and the fixtures list it. Grep the game's stdout for
  `Loading mod <name>`.

- **One case per blueprint string.** factorio-blueprint-editor's first probe put
  four cases in one string and got a single `import_stack` code of -1 for the
  lot, which could not be attributed. Worse, entities *did* come back, so -1
  looked like it might mean something other than what it does.

## Fields that do not mean what their name says

- **`tile_width` and `tile_height` are a centring parity, not a footprint.** The
  runtime docs say so outright: the field decides "if the center should be in the
  center of the tile (odd tile size dimension) or on the tile border (even tile
  size dimension)". For 146 of the 155 entities factorio-blueprint-editor knows,
  that coincides with the enclosing rectangle, which is exactly why reading it as
  a footprint looks right. For the other 9, every curved rail and `rail-ramp`,
  the published rectangle does **not contain the entity's own collision box**:
  `curved-rail-b` is 2x2 against a box 4.88 tiles tall, and `rail-ramp` is 2x16
  against a box 3.6 tiles wide. (#142)

  **A field whose name reads like a size is not therefore a size. Check what the
  docs say it decides.**

- **`support_range` is a load path, not a radius.** `RailSupportPrototype`
  declares 11 and `RailRampPrototype` 9, unchanged between the 2.0.73 and 2.1.12
  tags, and the docs give no units and no shape. One spot five tiles from a
  support, asked four times with only the rails between them changing, answers
  refused / accepted / refused / accepted. A lone support permits exactly the
  rails resting on it, and everything beyond has to be reached through rails that
  already exist. (factorio-blueprint-editor #141)

- **`can_place_entity` answers a different question per `build_check_type`.**
  `blueprint_ghost` skips rail adjacency entirely: a rail signal ghost is legal
  on bare grass hundreds of tiles from any rail, 2704 of 2704 in the #95 probe's
  empty-ground control, against 0 for `manual`. Use `manual` for "may this be
  built here". A probe that accepts either will read as "anything goes anywhere".

- **`create_entity` and `can_place_entity` disagree, and the gap is
  buildability, not collision.** `create_entity` will build an elevated rail on
  bare ground with no support beneath it; `can_place_entity` refuses one
  everywhere for exactly that reason. (factorio-blueprint-editor #133)

- **Ghosts are not a placement test.** Stamping a blueprint and reviving what it
  leaves looks like the most faithful way to ask whether the game accepts a
  layout. It is not: reviving one of two overlapping ghosts destroys the other
  whether or not the layout is legal, so a legal arrangement loses an entity
  exactly like an illegal one. #95 built and discarded that whole measurement.

- **"How far apart" has two answers when the thing is built incrementally.** A
  hand walk outward from one rail support stops at **12** tiles, because each
  rail has to be legal at the moment it is placed and the walk only approaches
  from one side. A **finished** line, built whole then knocked out and rebuilt,
  is legal to **20**, because the far half is reachable from the other support. A
  blueprint is a finished configuration, so 20 is the number that describes real
  exports, and every export in the corpus spaces supports exactly 20 apart.
  Measuring only the walk would have produced a rule that refuses every real
  elevated bridge. (#141)

## Version and seed traps

- **A version stamp is not enough on its own.** See method.md's fixture policy:
  `elevated-rail-collision.json` was captured on 2.1.12 for an editor targeting
  2.0.73, and one of twenty-one entries genuinely differs between those tags.

- **2.0.77 ships five bundled mods, not six.** No `recycler`. **Measured
  2026-08-17** on a standalone 2.0.77 install, and confirmed independently by the
  fixture-level diff between 2.0.77 and 2.1.14, where `recycler` appears in
  `loadedMods` only on the newer one. So a 2.0-versus-2.1 comparison can never be
  perfectly single-variable on mod set. That is a property of the game, not a
  setup error.

- **The Space-Age paths force a planet's surface seed to the map seed, which no
  real save does.** A planet generates at `mapSeed + crc32(planet.name)`, zero
  only for Nauvis. A forced-seed sample validates an expression port but is blind
  to seed plumbing, and the two disagree completely rather than subtly: 9.7% tile
  agreement. It cost a session to spot. (FactorioMapWebUI)

- **Factorio 2.1 deleted `LuaEntity.fluidbox` and the whole `LuaFluidBox`
  class**, flattening it onto `LuaEntity` as `get_fluid_box_*`. So
  `entity.fluidbox.get_pipe_connections(i)` became
  `entity.get_fluid_box_pipe_connections(i)`.
  `entity.prototype.fluidbox_prototypes[i].pipe_connections` still works in both,
  so prototype-level reads need no branching.

- **Feature detection needs a `pcall`.** Reading an unknown key on `LuaEntity`
  *raises* in 2.0 rather than returning nil, so the obvious guard throws on the
  older version:

  ```lua
  -- throws on 2.0
  if entity.get_fluid_box_pipe_connections then ... end
  -- works on both
  local has_new = pcall(function() return entity.get_fluid_box_pipe_connections end)
  ```

- **Re-capturing on a second version finds drift you were not looking for.**
  factorio-blueprint-editor re-took its entity footprints on a 2.0.x binary
  rather than 2.1.12, and four entities move footprint between the two:
  `tree-plant` and the three demolisher corpses. None was among the 155 that
  repo knows, so nothing broke - but nothing would have told them either. (#142)

- **Run a probe against two versions when you can.** The older one reproducing a
  known-good answer is what earns trust in the new one.
  `examples/pumpjack-terminals` does this, and it is how FactorioTools#81 was
  settled: all four rotations
  matched the planner on 2.0.77, which ruled out a broken probe, and two of them
  disagreed on 2.1.14.

## Numbers and determinism

These are specific to this tool, and each is load-bearing on `trim --check`
being a usable diff.

- **`serde_json` mis-parses long decimal literals by one ULP**, in both
  directions, on values like
  `0.394500000000000028421709430404007434844970703125`. **Measured 2026-08-17.**
  Round-tripping the 25 MB dump found 9,744 differing lines. The crate's
  *printer* is fine and its *parser* is not; Rust's own `f64::from_str` is
  correctly rounded and agrees with CPython bit for bit. The fix is
  `arbitrary_precision` plus re-parsing through `std`. Do not remove it.

- **`preserve_order` must stay off** so `serde_json::Map` is a `BTreeMap` and
  output sorts to match Python's `sort_keys=True`.

- **Sampled f32 values must round-trip exactly.** Shortest round-trip
  representation, never a fixed precision, never widened to f64. Scoring a port
  by exact-match count beats any error bound: two kernels shared an identical
  worst error of 2.682e-7 and differed by 42 exact matches of 512.
  (FactorioMapWebUI, `docs/noise/basis-noise-NOTES.md`)

## The lesson underneath all of these

**A fake can only be wrong in the ways its author already considered.**

Three real defects in this tool survived sixty unit tests and were caught by the
first run against the actual game: the sentinel read off the wrong stream, a
`mod-list.json` believed to load only `base`, and a seed hardcoded in `main` so
a spec's value went nowhere. Two of the three were invisible to the unit tests by
construction, because the fake encoded the same wrong belief the code did.

So make the fake wrong in the same way the real thing is, and settle any claim
about the game by running it.
