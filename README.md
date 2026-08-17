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

Practical notes, measured 2026-08-17:

- The feed is Atom, at <https://factorio.com/blog/rss>, served as
  `application/atom+xml`.
- It carries the **10 most recent posts only**. It is a what-changed-lately
  feed, not an archive, so it cannot answer "what shipped in version X" on its
  own.
- Each entry embeds the full post body, so recent posts need no second fetch.
- Older posts live at `https://factorio.com/blog/post/fff-<number>`.
- It is the only source here that needs the network. That is why it can never
  gate a capture: captures must stay reproducible offline and byte for byte.

## Examples

- [`examples/pumpjack-terminals`](examples/pumpjack-terminals) - a `create` probe
  that places pumpjacks in all four rotations and asks the game where the pipe
  goes. Run against 2.0.77 and 2.1.14, it settled FactorioTools issue #81 and
  caught a breaking runtime API change on the way.

## Scope

Behavioural reverse engineering for interoperability - understanding what the
game computes so other projects can agree with it. Not extracting or
redistributing game code or assets. Keep it that way.
