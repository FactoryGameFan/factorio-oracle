# Where does a pumpjack's pipe actually go?

A worked example of `mode: "create"`. It places pumpjacks in all four rotations
and asks the running game where the connecting pipe goes, rather than working it
out from prototype data.

```bash
factorio-oracle run --probe examples/pumpjack-terminals/probe.json --work-dir /tmp/pj
cat /tmp/pj/write/script-output/oracle-dump.json
```

## Why it exists

FactorioTools hardcodes four pipe offsets, one per pumpjack rotation. A
`--dump-data` capture shows the prototype's `output_fluid_box.pipe_connections`
changing between versions, but it does not say where the *pipe* goes. Those are
different questions: `positions` is the connection point inside the entity, and
the pipe sits one tile further out in the direction the entity faces.

Deriving one from the other means trusting a convention. `target_position` is the
answer with no derivation in it, so this probe reads that.

## What it found

Run against real 2.0.77 and 2.1.14 installs on 2026-08-17. Offsets are relative
to the pumpjack's center tile.

| Facing | 2.0.77 | 2.1.14 |
| --- | --- | --- |
| north | `(1,-2)` | `(1,-2)` |
| east | `(2,-1)` | **`(2,1)`** |
| south | `(-1,2)` | `(-1,2)` |
| west | `(-2,1)` | **`(-2,-1)`** |

2.0 gave four rotations only two distinct corners: east reused north's and west
reused south's. 2.1 gives each rotation its own corner. FFF #442, "Flip, Flow,
and Fresh Paint", is the release that did it, along with `use_mirroring` and
`migrate_horizontal_mirroring` on the same prototype.

Running both versions is what makes the result trustworthy. The 2.0.77 numbers
match what FactorioTools has hardcoded, so the probe reproduces the known-good
answer before being believed about the new one. A probe that only runs against
the version you care about cannot tell you that.

Also read from the running game, and identical on both: `defines.direction` is
`north=0, east=4, south=8, west=12`.

The 2.1.14 row was produced twice, on a Windows standalone install and on a
macOS Steam install of the same build, and the numbers match. That is consistent
with `--dump-data` being byte-identical across those two platforms for a given
build, which was measured the same day.

## Two traps this probe hit

**Factorio 2.1 deleted `LuaEntity.fluidbox` and the whole `LuaFluidBox` class.**
It is flattened onto `LuaEntity` as `get_fluid_box_*`:

```lua
-- 2.0
entity.fluidbox.get_pipe_connections(index)
-- 2.1
entity.get_fluid_box_pipe_connections(index)
```

`entity.prototype.fluidbox_prototypes[i].pipe_connections` still works in both,
so prototype-level reads need no branching.

**Feature detection needs a `pcall`.** Reading an unknown key on `LuaEntity`
*raises* in 2.0 rather than returning nil, so the obvious guard throws on the
older version:

```lua
-- throws on 2.0
if entity.get_fluid_box_pipe_connections then ... end
-- works on both
local has_new = pcall(function() return entity.get_fluid_box_pipe_connections end)
```

## Notes on the shape

- `on_init` is fine here because this mod owns its own `control.lua`. A shared
  prelude could not use it, since `script.on_init` takes exactly one handler.
- No `error("DUMPED-OK")` sentinel. `--create` exits 0 on its own, and create
  mode is judged by whether the dump exists.
- Pumpjacks need oil under them, so the probe lays a 3x3 `crude-oil` patch first,
  and calls `force_generate_chunk_requests()` because the placements sit outside
  the small area a fresh map generates.
