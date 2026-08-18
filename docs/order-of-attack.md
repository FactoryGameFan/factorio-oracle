# Order of attack

Which source to ask first when you need to know what Factorio does. This is the
part that saves the most time, and it is not "open a disassembler".

Lifted from `factorio-blueprint-editor`'s `tools/oracle/README.md`, where it was
written after the fact, once the expensive ordering had already been tried.

## 1. factorio-data first

Most behaviour is Lua, shipped in the clear at
[github.com/wube/factorio-data](https://github.com/wube/factorio-data), with one
git tag per release. It is free to read, it needs no install, and it answers
more questions than people expect.

**Grep for the definition site, not a bare name.** `name *= *"<thing>"` finds
where a prototype is declared. A bare name matches every caller, which on a big
prototype can be hundreds of lines of noise.

**Check out the tag that matches what you target, not the newest one.** The four
consumer repos target different versions on purpose, and a rule grounded against
the wrong tag can be right about a game nobody is running.
factorio-blueprint-editor learned this the hard way: it targets 2.0.45 to
2.0.73, but its blueprint corpus spans 2.0.32 to 2.1.12, and two of its twelve
files declare 2.1.12. A rule checked only against the 2.0.73 tag predates those
two files.

**A clone is not a checkout.** `~/GitHub/factorio-data` is one working tree that
three repos want at three different tags. Read at a tag without moving `HEAD`
(`git show <tag>:<path>`, `git grep <pattern> <tag>`), or take a worktree. Moving
`HEAD` in a shared clone breaks whatever else is reading it, silently.

**The migration files are the same bytes as an install's.** Verified 2026-08-17
across all 16 `.json` files under `data/base/migrations` and
`data/space-age/migrations` on 2.1.14: `diff -rq` against a real install reports
no differing file. So a rename can be checked with no Factorio installed at all.

## 2. Then the oracle

This tool. Use it for anything the Lua does not answer because it is engine
behaviour rather than data: import rules, validation, placement, numeric
primitives, and every `defines` value.

The dividing line is whether the answer is written down anywhere. If it is in
`data.raw`, the migrations, or `runtime-api.json`, read it. If the only way to
learn it is to make the game do the thing and watch, that is a probe.

**Check whether a prototype field answers it before building a placement rig.**
factorio-blueprint-editor's `probe-rail-placement.mjs` skipped elevated rails
because placing one needs rail supports, and it was asking a placement question.
The question underneath was collision, and `LuaEntityPrototype.collision_mask`
needs nothing placed at all. Sweeping the prototype table gave the complete set
rather than a list written in advance.

## 3. Only then the binary

Factorio ships unstripped, so a short list of genuinely compiled things can be
recovered from it. FactorioMapWebUI has done this for the noise generator's
gradient table, which no formula reproduces.

Nothing in factorio-blueprint-editor has needed it yet, after eighteen probes.
Treat reaching for it as a signal that steps 1 and 2 were not exhausted.

## Two sources that answer a different question

`README.md` splits sources into two kinds, and the split decides how each may be
used.

**Authoritative** sources say what the game accepts. They ship with the install,
read offline, and are byte-identical run to run: `--dump-data`,
`data/*/migrations/*.json`, and `doc-html/runtime-api.json`. Only these may gate
a capture.

**Reference** sources say *why* something changed, which is what tells you which
captured value now needs review: `data/changelog.txt`, and the Friday Facts blog
at <https://factorio.com/blog/>. Nothing automated reads them and nothing should.
The blog is Wube's own writing, so it outranks forum and wiki advice, but it
describes intent, and the shipped game can differ. Check it against a capture.

Search the blog at `https://factorio.com/blog/search/<term>`, which covers the
whole archive. Word choice matters: `mirroring` does not return FFF #442, the
post that introduced entity mirroring, while `flip` and `fluid` both do. Try two
or three wordings before concluding a topic was never written about.
