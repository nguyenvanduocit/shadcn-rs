# 0030. A trial previews an input

Status: proposed (M17); not accepted, not built. Split from ADR-0028 on 2026-11-24, after three review rounds by two
model families failed on it. This record is a problem statement with constraints, not a design. It needs its own
acceptance before anything is built.

## Context

**The goal.** Run an input in a fork, with real handlers, and see what it would do, without doing it:

- an agent is shown the result of an action before it is real (frontiers §5);
- a replay check: an input run twice gives the same result, the referee of ADR-0023 applied to inputs (frontiers §4).

Predicting a frame to cut input latency (frontiers §3) is a third use; it needs a fork's *result*, not only its
observation, and is further away.

**What is settled, in ADR-0028.** A fork is always rolled back. A `Check` runs no handler, works on a detached tree
and is safe in every app. The outbox holds every effect posted inside a fork and drops it. The registry lists the
engine's state.

**Why this is harder than a check.** A check runs code that must not change state: a write is a fault. A trial runs
code whose purpose is to change state, and then has to un-change all of it, including state the engine cannot see.

## The design so far

Not decided; this is where the three rounds ended.

- **A trial never commits.** A committed fork must be exactly a live frame, and two rounds failed on every place it
  could differ (ADR-0028, option 3).
- **Applying means replaying.** The input is delivered again, live, by the ordinary path. The live result is
  compared with what the trial observed, and a difference is reported.
- **Opt-in per app root** (`App::trials()`), because handlers are app code. The contract: state that app closures
  read lives in signals.
- **The contract is checked by a double run:** the trial runs twice from the same start; if the two observations
  differ, some state was hidden.

## Open problems

Each has a scenario in which the design above gives a wrong answer, and the direction it points to.

1. **No shared start point.** A trial runs at one moment; the live replay runs later. Between them a mailbox
   delivers, a timer fires or another input arrives, so the live run starts from different state and the comparison
   reports a difference that is nobody's fault. *Direction:* an epoch on the live state; an observation is stale
   once any live turn has run, and a stale observation is discarded, not compared.
2. **Time and randomness are not recorded inputs.** A handler reads the clock or draws a random id. The trial, the
   second run and the live replay each get another value, so both checks fail on correct code. *Direction:* the
   clock and the random stream are recorded with the turn and replayed. This is why components read them only
   through `gui::clock` and `gui::random` from K4a.
3. **Engine cleanups do not run in a fork.** Closing a dialog in a trial should pop its layer, release its capture
   and return focus; those are cleanups today, and ADR-0028 runs none. The trial's frame then shows a state no live
   frame reaches. *Direction:* engine cleanups become typed, logged steps that run in a fork and are undone; app
   cleanups stay un-run, and what an app cleanup would have written is a stated difference.
4. **What is compared, and where it ends, is undefined.** A handler posts a clipboard write and continues in the
   answer; a `spawn` continues in `then`. In a trial no answer comes, so the trial stops at the post while the live
   run goes on. Comparing frames then always differs. *Direction:* either the comparison ends at the posts (signals
   written and effects posted, in order, up to the first answer awaited), or answers are simulated from a declared
   model of each port.
5. **Writes in place fall outside the log.** `Signal::update(&mut T)`, `Collection` and `Holder` steps, changes to
   the retained tree, fields of `ListState`. A log of `set` sees none of them. *Direction, two candidates:* a log
   over value writes (clone on the first mutation in a fork, which is why `Signal::new` asks `T: Clone`; structural
   steps logged and undone), with a structural undo log on the retained tree; or a detached trial tree, as the check
   has, with the live state copied on write. The first changes the live tree and must restore it exactly; the second
   must define how handlers find nodes of a tree that is not the live one.
6. **A latch or a `watch` hides history.** A `watch` keeps its previous value; an `FnMut` handler keeps a counter; a
   "fire once" flag is set in the trial and stays set, so the live replay does nothing. The double run sees this
   only if the second run's outcome differs, and by then the live closure is already changed. *Direction:* the
   previous value of a `watch` becomes engine state; captured `Rc<Cell>` and `Signal<Cell>` are linted out of our
   crates (K4a); for app closures the limit is stated, or handlers are `Fn`.
7. **`FnOnce` hooks are consumed.** An `after_layout` or `on_mounted` hook that runs in a trial is gone when the
   live frame needs it. *Direction:* a trial does not run hooks and the comparison ends before them, or a hook is
   taken out under the log and put back.
8. **Payload equality is undefined.** Comparing two observations needs to compare posted messages and written
   values, which have no `PartialEq` bound. *Direction:* compare by port, order and count only; or ask messages for
   a digest; or compare what the frame shows and nothing else.
9. **Pending work.** A live frame runs pending marks, hooks and due timers before the input's own work. A trial
   that starts with empty queues (Fresh, as a check has) skips them; one that starts with the live queues must put
   them back in order. *Direction:* queues and timers forked with a log, for trials only.
10. **Reads of the outside inside a trial.** A copy followed by a paste in one trial should read the copy.
    *Direction:* the outbox overlays reads of what it holds; a read of the outside is recorded as an input.
11. **A direct effect is invisible to every check.** A file written twice looks the same to the double run.
    *Direction:* the limit is stated; the clippy bans cover our crates; `gui::trial::refuse(reason)` lets a handler
    end a trial.

## Constraints on any answer

- **No commit.** A trial is rolled back; applying is a live run.
- **The check of ADR-0028 does not change.** It stays on a detached tree, runs no handler and needs no opt-in.
- **No component changes.** The component-visible API landed at K4a for this reason: `Port`, `spawn`,
  `Sender::send -> Result`, `clipboard::write` with a context and an answer port, `Callback::call -> Option<R>`,
  `Signal::new` with `T: Clone`, `gui::clock` and `gui::random`, and the lints on signal types, captured `Rc<Cell>`,
  the system clock and direct effects. An answer that needs more than these from a component must say why.
- **Zero cost when no fork is open,** by ADR-0028's two-part gate.
- **A trial is never the default.** An app that has not opted in runs none.
- **Every guarantee is checked or stated as a limit.** A declaration by the author of a handler is not a check.
- **`panic = "abort"`** gives no trial, as it gives no check.

## Consequences of leaving it open

- Agent preview, the replay check and frame prediction wait for this record. The sampled check does not.
- S8 is written once against the K4a API, so accepting this record later is engine work.
- If the answer needs queues to fork or the retained tree to carry a log, ADR-0028's registry table and ADR-0026's
  commit gain them then; neither is built before.

## First experiment

Take problem 4 first, because it fixes what a trial is for. Write down, for three real inputs (a click that closes a
dialog, a cut in an input, a key that scrolls a windowed list), the observation an agent would want and the point at
which a live run stops matching it. The other problems are then read against that definition.
