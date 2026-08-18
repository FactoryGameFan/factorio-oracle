# Method

How to ask the game a question and get an answer you can trust. These are
transferable rules, not Factorio facts. For the facts, see
[gotchas.md](gotchas.md).

Every rule here was paid for by a probe that produced a clean, confident, wrong
answer. The examples name the repo and issue so you can go read the wreckage.

## A control must be able to fail while the hypothesis holds

This is the one that catches the most, and it is the easiest to get wrong,
because a control that cannot fail still passes and still feels like rigour.

factorio-blueprint-editor's rail-on-rail probe (#133) used "a rail on an
identical rail overlaps nowhere" as its second control. It failed on every
curved orientation: 4 overlapping spots per `curved-rail-a` orientation, 8 per
`legacy-curved-rail`. Nothing was actually wrong. A curved rail's tile rectangle
is mostly empty, so a second identical curved rail sits legally beside it with
the rectangles overlapping and the curves not.

The problem was that the control restated the hypothesis. It could only agree or
announce the finding, never invalidate the apparatus. It was replaced with "the
anchor's own position is never accepted", which is about the rig.

**The null-result form is worse**, because nothing looks wrong. In PR #222, four
of seven cases would have reported "the coordinates did not change" whether the
probe was right or reading the same field twice. What rescued it was a
`shifted-entities` case that moves two chests three tiles left and four up
without touching any snapping property. Its coordinates *have* to differ. Without
a control that can fail, "nothing moved" is not evidence of anything.

## Include a control that can invalidate the probe itself

Not a control for the behaviour. One for the measurement.

factorio-blueprint-editor's `probe-copy-settings-schedule.mjs` (#115) runs a case
where `copy_settings` is never called, because `create_blueprint` groups a
train's locomotives into one `schedules` entry on its own. Without that case, a
merged entry would have looked like a successful copy.

The filter-count probe (#93) learned the same lesson the expensive way. It read
50 as a cap when it was the game deduplicating 50 cycled item names.

**Report a section its control voids as unmeasured.** When
factorio-blueprint-editor's elevated-rail sweep came back 0 accepted against a
0-of-8 empty-ground control, the zeros were not a finding. The section had voided
itself.

## Last man standing is not a measurement

Evaluate every probe against every rule, and report the rules that survive **all**
of them, not the one that fits the last result.

The zoom probe (#206) had three candidate rules for what a notch does from an
off-ladder start. All three agree on every sample taken from a rung, so only an
off-rung start says anything, and *which pair* it separates depends on where in
the gap it sits. A start at 0.83 sits below its gap's midpoint, so a notch **in**
rules out "multiply where you are" and cannot tell "snap to the nearest rung then
step" from "move to the next rung in the direction of travel". Both say
0.905724, exactly. A notch **out** from the same value separates them, 0.742997
against 0.820335, and settled it.

Reading the first probe as a verdict would have adopted the wrong rule with a
number matching to nine digits.

**Which hypotheses a probe value can separate is part of the question.** Decide
that before you run it, not after.

## Refute the rival

A rule that explains your data is not the same as a rule your data picked out.
When two rules both fit, the useful probe is the one designed to kill one of
them, not the one that confirms both again.

This is the same discipline as the section above, applied before the probe is
written rather than after it returns.

## A probe entity is part of the question, not a neutral instrument

"Which tiles does this rail occupy" sounds like a property of the rail. It is
not. Collision is continuous, so the answer depends on how big the thing you ask
with is.

factorio-blueprint-editor measured this with four 1x1 references (#133). A
`small-electric-pole` (0.3 x 0.3) fits on cells a `wooden-chest` (0.7 x 0.7) is
refused on, and a `transport-belt` (0.8 x 0.8) is refused on cells the chest is
not: 28 and 24 of 38 rail orientations respectively.

**Sweep more than one reference before concluding anything of the form "X
occupies Y". If they disagree, that is the finding.**

**A probe entity's own lattice counts too.** The #141 probe put a `rail-support`
at the coordinates of the rail it was meant to hold, because both are
`build_grid_size = 2` and it looked like one lattice. It is not. A north-south
line of `elevated-straight-rail` sits on odd y and its supports on **even** y,
between two rails, and which parity is legal depends on the support's
orientation. A support on the wrong parity is created happily, stands there, and
holds nothing. Every profile came back byte-identical to having no support at
all, across all 16 orientations and 16 distances. That reads as "supports do not
participate in buildability", which is clean, confident and wrong.

Sweeping both parities of both axes cost four lines. What actually caught it was
not the probe: it was decoding a real export from the corpus and looking at where
the game had put the supports.

**When a probe says an entity does nothing, check a real example of it doing
something before believing the probe.**

## Ask the cheapest question that settles it

Before building a rig, check whether something simpler answers the same question.
A prototype field beats a placement test. A read beats a sweep. A sweep of a
table beats a list written in advance.

`probe-rail-placement.mjs` skipped elevated rails because placing one needs rail
supports. The question underneath was collision, and
`LuaEntityPrototype.collision_mask` needs nothing placed at all.

## Transcribe the rule into the probe before writing any code

Put the rule you are about to implement into the probe, and report what it would
get wrong across every measured row.

factorio-blueprint-editor's rail-on-rail probe carries two transcriptions of the
editor's arms, before and after, and scores both. The first draft of a fix,
"allow whenever the prototypes differ", produced four corruption-class rows. The
re-run caught it in eight seconds, and no test would have suggested it.

**A transcribed rule needs a coordinate control, not only a logic one.**
`probe-entity-tile-size.mjs` compares two candidate footprints against a fixture
whose blocked cells are stored relative to `floor(position)` rather than in world
coordinates. The first transcription keyed its rectangles absolutely. Both arms
inflated together, 440/356 against 435/399, which preserved the verdict and
looked entirely plausible. Nothing about the shape of the output said it was
wrong.

What says so is a row whose answer is known independently. A cardinal
`straight-rail` is exact on both arms, so it must come back all zeros, and that
is asserted.

## Sweep two window sizes, and make the wider one an explicit control

`probe-rail-placement.mjs` swept plus or minus 3 tiles. That is ample for a 2x2
rail and not for a 4x8 one. `legacy-curved-rail`'s legal signal positions reach
an offset of 3.5, so the fixture lost one or two of the four at every orientation
and recorded 2 or 3.

Nothing in the output looked wrong. The spots it did find were real.

The fix is to sweep at two window sizes and make "the wider one finds nothing
new" an explicit control, which is what `probe-rail-signal-spots.mjs` does. It
caught 16 clipped sweeps of 76.

The rule built on the clipped number survived re-checking, so the cost was a
wrong count rather than wrong behaviour. That is luck, and it was only knowable
by re-measuring.

## Ask for the whole answer, not the one value you brought

The #95 probe asked whether a gate was placeable at one direction, the one its
control had picked while standing in open ground, and got a false negative
because that direction was parallel to the rail. Sweeping all sixteen turned it
into an 8-versus-0 finding.

**If a control fails, suspect the question first.**

## Ask what a limit is before comparing it to a number

`zoom_limits` has three fields, not two, and they are not all the same kind of
thing. `closest` is a zoom. `furthest` is a **rule**: at most 200 tiles across
the window, capped at 500. Comparing that to a readback answers "DISAGREE" for a
limit that is perfectly correct.

Transcribing the documented formula turned an incomparable field into a third
instrument that agrees with the readback to 1e-9. The third field,
`furthest_game_view`, was not known to the design at all, and is the one that
matters to a viewer with no map view.

## A script-set value is not a measured one

The command that sets an off-ladder zoom trips the live sampler on the same tick,
so 0.83 arrives looking exactly like a rung the wheel stopped on. It would join
the distinct set, sit between two real rungs, and break the constant-ratio check
for a reason that has nothing to do with the game.

**Tag what the probe itself wrote, and exclude it by tag rather than by value.**

## "No headless probe can reach it" is not "not worth measuring"

The zoom design (#206) wrote the per-notch step off as "an input-handling
constant no headless probe can reach", and proposed shipping a guessed 1.33.

The real answer is `2^(1/7)`, three notches of the real ladder away from the
guess, and getting it took one sitting with a human at the keyboard.

Some questions need a person. That is a cost, not a wall. The first claim was
allowed to stand in for the second, and it should not have been.

## Be ready for the measurement to refuse the change it was capturing evidence for

This has happened four times in factorio-blueprint-editor, which is often enough
that it should be an expected outcome rather than a surprise.

- **#142**, "make the editor's footprints agree with the game's `tile_width`".
  They already agreed for 146 of 155 entities, and adopting the numbers made
  agreement with measured occupancy worse in both directions.
- **#206**, "use the game's zoom range". 30 of 367 corpus blueprints are wider
  than the game's 200-tile floor allows, so adopting it makes 8% of real
  blueprints impossible to view whole. The game's limit exists to stop a player
  seeing ungenerated chunks, which an editor does not have to care about. Its
  map editor floor, 0.1, is the number that transfers.
- **#141**, where the measured half turned out not to be a geometry at all.
- **PR #222**, where the premise was that a blueprint's grid position moves its
  entities. It does not.

A probe that only ever confirms the plan is not being pointed at anything.

## Fixture policy

Capture once, commit the JSON with its provenance, assert offline. Shared by both
sibling repos, and enforced here by `factorio-oracle provenance check`.

- **Never hand-edit a fixture to make a test pass.** A mismatch is a finding.
- **Version-stamp every capture.** Steam updates the binary without asking.
- **A fixture's provenance is a record of the moment it was captured, not a live
  claim.** Never edit one to make it current. factorio-blueprint-editor has six
  fixtures carrying a `versionCaveat` that was true when written and is not any
  more; the right fix is to correct the probe so a future recapture states it
  properly, not to rewrite history.
- **A probe that owns a fixture recaptures it behind a flag, never on every run.**
  A probe that rewrote its own fixture would turn "the game changed" into "the
  fixture changed".
- **Record a cross-check, not only the binary.** `elevated-rail-collision.json`
  was captured on 2.1.12 for an editor targeting 2.0.73. Cross-checking against
  the Lua at the 2.0.73 tag found one real difference in twenty-one entries:
  `core/lualib/collision-mask-defaults.lua` has `["cargo-bay"] = building_tall()`
  at 2.0.73, which carries the elevated rail layer, and `building()` at 2.1.12,
  which does not. A fixture saying only "2.1.12" would have been correct and
  still produced the wrong rule.
