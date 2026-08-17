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

## Scope

Behavioural reverse engineering for interoperability - understanding what the
game computes so other projects can agree with it. Not extracting or
redistributing game code or assets. Keep it that way.
