# Lore

This directory captures non-trivial discoveries: things that took real
effort to figure out and aren't obvious from reading the code or the
architecture docs. Think of it as tribal knowledge that would otherwise live
only in someone's head or get re-discovered painfully by the next person
(human or agent) who hits the same thing.

## What belongs here

- A CMake behavior that was surprising or under-documented.
- A Bazel rule/toolchain quirk that cost time to track down.
- Why a previously-tried approach was abandoned, and what specifically went
  wrong with it.
- Any "if you don't know this, you will waste an afternoon" fact.

## What doesn't belong here

- Design decisions and current architecture — that's
  [docs/architecture/](../architecture/).
- Step-by-step instructions for resolving a specific translation gap —
  that's a [runbook](../runbooks/).
- Anything easily re-derived by reading the current code.

## Format

One file per discovery, named for the topic
(e.g. `cmake-generator-expressions-in-custom-commands.md`). Keep entries
short: what you hit, why it's surprising, what the resolution or workaround
was. Link to related architecture docs or runbooks with relative links
where useful.
