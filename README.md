# factorio-oracle

Asks a real Factorio install what it does, so behaviour that other projects
reimplement can be checked against the game rather than against assumptions.

Four repos each wrote this plumbing separately: FactorioTools,
factorio-blueprint-editor, FactorioMapWebUI and FactorioWikiDamageThresholds.
This is that plumbing, once.

## What it does and does not do

It owns discovery, mod scaffolding, launching and reading results back. It owns
none of the analysis. A probe compares the game against a consumer's own
reimplementation, so that half has to run in the consumer's language. The
interface is therefore JSON in and JSON out, not a probe framework.

## Sources

Two kinds, and the difference decides how each may be used.

**Authoritative sources** say what the game accepts. They all ship with the
install, so a capture reads them offline and gets the same bytes every time.
These are the only things a capture may depend on.

| Source | Answers |
| --- | --- |
| `factorio --dump-data` | Every prototype: names, collision boxes, pipe connections, pole supply and wire reach, beacon stats |
| `data/*/migrations/*.json` | Every rename, as a table. A complete list, not a guess |
| `doc-html/runtime-api.json` | The shape of the `defines.*` tables. It publishes no values, so read those with a probe instead |

**Reference sources** say why something changed. They are for a human reading
after a game update, to work out which captured value now needs review. Nothing
automated reads them, and nothing automated should.

| Source | Answers |
| --- | --- |
| `data/changelog.txt` | Behaviour changes per patch, terse, ships with the install |
| <https://factorio.com/blog/> | Wube's own Friday Facts posts: the reasoning behind a change, usually weeks before it ships |

### On the Friday Facts blog

It is official, written by the developers, so it outranks forum, wiki and blog
advice from anyone else. It is still not authoritative in the sense above: it
describes intent, and the shipped game can differ. Check any claim from it
against a capture before acting on it.

A worked example. Factorio 2.1 changed the pumpjack's `output_fluid_box`
pipe connections from two distinct corners repeated, to four distinct corners,
one per rotation:

```
2.0.77  positions = [[1,-1], [1,-1], [-1,1], [-1,1]]
2.1.14  positions = [[1,-1], [1,1],  [-1,1], [-1,-1]]
```

The dump shows the change. It does not say why. FFF #442, "Flip, Flow, and
Fresh Paint", is the post that explains it, and the same release added
`use_mirroring` and `migrate_horizontal_mirroring` to that prototype. Reading it
is how you learn the change is about entity mirroring rather than a typo fix.

**Search is the way in**: `https://factorio.com/blog/search/<term>`. It covers
the whole archive, not just recent posts - a search for `mirroring` returns
FFF #80, from 2015.

Practical notes, measured 2026-08-17:

- Search returns HTML, not JSON, with about 10 results and no total shown.
- **Word choice matters more than you would expect.** Searching `mirroring`
  does *not* return FFF #442, the post that introduced entity mirroring, while
  `flip` and `fluid` both do. Try two or three wordings before concluding a post
  does not exist. A bare number works too: `search/442`.
- Direct post URLs are `https://factorio.com/blog/post/fff-<number>`.
- There is an Atom feed at <https://factorio.com/blog/rss>, served as
  `application/atom+xml`, with each entry embedding the full post body. It
  carries the **10 most recent posts only**, so it answers "what changed lately"
  rather than "what shipped in version X". Use the search for that.
- The blog is the only source here that needs the network. That is why it can
  never gate a capture: captures must stay reproducible offline and byte for
  byte.

## Provenance

A fixture that does not say which Factorio produced it is a number with no
claim attached. Provenance records that, in a `PROVENANCE.json` beside the
fixtures rather than inside them: most fixtures are verbatim copies of the
game's own JSON, asserted byte for byte, so a metadata key added inside one
would be data pollution.

```bash
# Structural check. No Factorio needed. Exits 1 on a finding.
factorio-oracle provenance check tests/fixtures

# Version comparison. Needs an install. Always exits 0.
factorio-oracle provenance report tests/fixtures
```

Enforcement splits in two on purpose. `check` answers questions a machine can
settle - is every file recorded, does every entry name a file that exists, is
every entry well formed, and has the `unknown` count grown - so it runs in CI
with no game. `report` answers a question a machine cannot: a fixture captured
on 2.1.11 is not wrong because the binary moved on, it just has not been
re-validated since, and whether that matters depends on whether the subsystem
changed. So it never fails a build.

Two required keys per entry, `factorioVersion` and `evidence`. Any other key is
carried through and never validated. `evidence` is free text; the grade that is
enforced is `factorioVersion: "unknown"`, and `maxUnknown` caps how many entries
may say it. That number is a ratchet, not a cap: the check fails if the count
rises above it and also if the count drops below it and the number is not
lowered.

Every file in the tree needs an entry. A file that is deliberately not ground
truth goes in `notFixtures` with a reason. There is no extension filter,
because an ignore rule that costs nothing gets used without thinking.

**A fixture's provenance is a record of the moment it was captured, not a live
claim.** Never edit one to make it current, and never edit one to make a test
pass. A mismatch is a finding.

## Reference material

Most questions about Factorio are answered by Lua that ships in the clear at
`github.com/wube/factorio-data`, one git tag per release. The catch is that
one clone has one working tree, and several repos want several versions of it
at once. Checking a tag out is how one repo silently breaks another's read.

So this reads at a tag and never moves `HEAD`.

```bash
# One file at a tag.
factorio-oracle refs show 2.0.77 base/info.json

# Search one tag. Output is git grep's, with the <tag>: prefix removed.
factorio-oracle refs grep supply_area_distance --tag 2.1.14

# Search several, and find out whether the answer moved.
factorio-oracle refs grep support_range --tag 2.0.73 --tag 2.1.12 \
  --path elevated-rails/prototypes/entity/elevated-rails.lua

# A real directory, for ripgrep or an editor or a Lua parser.
cd "$(factorio-oracle refs worktree 2.0.77)"
factorio-oracle refs worktree 2.0.77 --remove

# One Lua API docs file. An installed game answers with no network.
factorio-oracle refs docs 2.1.14 runtime-api.json --which

# Can this version be read at all?
factorio-oracle refs sync 2.0.73
factorio-oracle refs sync 2.0.73 --check   # exits 1 if not
```

The clone is `~/GitHub/factorio-data`, or whatever `FACTORIO_DATA_DIR` names.
Worktrees and fetched docs go under `~/.cache/factorio-oracle`, or
`FACTORIO_ORACLE_CACHE`.

**The multi-tag verdict is the reason this exists.** "Is this value still the
same two versions later" is a question every consumer repo has answered by
hand, by checking a tag out or reading two web pages, and then written the
answer into a fixture as prose. Two greps answer it, and the comparison
ignores line numbers on purpose: a value that moved down the file has not
changed.

**`sync` reports availability, not state.** Since nothing here checks anything
out, there is no pinned working tree to keep in sync and no lock file to go
stale. `sync` answers "is the clone here, is the tag fetched, are the docs
reachable", and may fetch tags to make the answer yes. `--check` never fetches
and never writes.

**Docs come from an installed game first.** Measured on 2.1.14: the install's
`doc-html/` is byte-identical to the published archive's contents across all
3,370 files, and the install even ships that archive itself, with the same
sha256. So for a version you have there is nothing to download. For a version
you do not have, one file is fetched and cached - `runtime-api.json` at 2.0.45
is 1.6 MB against 24 MB for the whole archive.

The limit that follows, stated rather than hidden: **you cannot search a
version nobody has installed**, because you cannot grep files you never
fetched. Install that version, or use `refs grep`, which searches the Lua
rather than the docs.

## Examples

- [`examples/pumpjack-terminals`](examples/pumpjack-terminals) - a `create` probe
  that places pumpjacks in all four rotations and asks the game where the pipe
  goes. Run against 2.0.77 and 2.1.14, it settled FactorioTools issue #81 and
  caught a breaking runtime API change on the way.

## Writing a probe

Three documents carry what probe-writing has cost across the four repos so far.
They are prose for a person, not input to anything.

- [`docs/order-of-attack.md`](docs/order-of-attack.md) - which source to ask
  first. factorio-data, then the oracle, then the binary, and why that order
  saves the most time.
- [`docs/method.md`](docs/method.md) - how to design a probe whose answer you can
  trust. Controls that can fail, refuting the rival rule, and why a probe entity
  is part of the question.
- [`docs/gotchas.md`](docs/gotchas.md) - specific facts about Factorio and its
  API, each of which cost a failed run.

Lifted with attribution from `factorio-blueprint-editor` and `FactorioMapWebUI`,
which learned most of it the expensive way. That borrowing had been done by hand,
repo to repo, and drifted; this gives it one home.

## Scope

Behavioural reverse engineering for interoperability - understanding what the
game computes so other projects can agree with it. Not extracting or
redistributing game code or assets. Keep it that way.
