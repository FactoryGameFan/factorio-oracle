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

## Examples

- [`examples/pumpjack-terminals`](examples/pumpjack-terminals) - a `create` probe
  that places pumpjacks in all four rotations and asks the game where the pipe
  goes. Run against 2.0.77 and 2.1.14, it settled FactorioTools issue #81 and
  caught a breaking runtime API change on the way.

## Scope

Behavioural reverse engineering for interoperability - understanding what the
game computes so other projects can agree with it. Not extracting or
redistributing game code or assets. Keep it that way.
