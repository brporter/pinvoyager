# Phase plans

Work on PinVoyager is organized into numbered phases. Each phase has one document here.

These documents are the interface between you and whoever executes the work. They exist so
that a phase can be handed over and carried out largely independently, without a running
conversation to fill in gaps.

## Index

| # | Phase | Status | Document |
|---|---|---|---|
| 1 | Design | complete — reviewed and approved | [phase-01-design.md](phase-01-design.md) |
| 2 | Backend foundation | complete | [phase-02-backend-foundation.md](phase-02-backend-foundation.md) |
| 3 | Authentication | in progress — blocked on credentials | [phase-03-authentication.md](phase-03-authentication.md) |
| 4 | Photo pipeline | complete | [phase-04-photo-pipeline.md](phase-04-photo-pipeline.md) |
| 5 | Voyages API | complete | [phase-05-voyages-api.md](phase-05-voyages-api.md) |
| 6 | Frontend shell | not started | [phase-06-frontend-shell.md](phase-06-frontend-shell.md) |
| 7 | Map experience | not started | [phase-07-map-experience.md](phase-07-map-experience.md) |
| 8 | Discovery & polish | not started | [phase-08-discovery-polish.md](phase-08-discovery-polish.md) |

Phases 2–8 are first drafts. They are deliberately unfinished — sharpen the targets of a
phase before starting it, not all of them up front, because what you learn in phase 3 will
change what you want from phase 4.

## Working agreement

**You own the targets.** A phase's `## Targets` and `## Acceptance criteria` sections are the
contract. Edit them freely. If a target is in the document, it gets built; if it isn't, it
doesn't. Vague targets produce vague work, so prefer "returns 413 above 25 MB" over
"handles large uploads well."

**Handing over a phase** means saying which phase to start. The expectation is then:

1. Read `docs/decisions.md` and the phase document.
2. Ask about anything under `## Open questions` that would change the work — before building,
   not after.
3. Build every target. Not a subset.
4. Verify against the acceptance criteria and report what actually happened, including
   failures.
5. Update `## Status` here and in the phase document.

**Scope is not silently negotiable.** If a target turns out to be wrong or blocked, the right
move is to build everything else, then say plainly what was left and why. Deciding to do less
is your call, not the executor's.

**Decisions live in `docs/decisions.md`.** Anything settled there is settled. A phase document
that contradicts it is a bug in the phase document.

## Conventions

- **Status** is one of `not started`, `in progress`, `complete`, `blocked`.
- **Targets are numbered** and referred to by number (`T3`) in commits and discussion.
- **A phase is complete** when every acceptance criterion passes, not when the code is written.
- **Design references** point at files in `design/`. If a phase implements a screen, the
  screen exists in the design system first.
