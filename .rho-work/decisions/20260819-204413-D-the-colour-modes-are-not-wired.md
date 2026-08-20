# D-the-colour-modes-are-not-wired — the claim is corrected, not the code

Date: 20260819

## The question

`F-theme-roles` said six roles each resolve to a 256-colour value, a 16-colour fallback, and a
no-colour modifier set. A test-quality audit found that the renderer reads none of the last two. Is
that a defect to fix now, or a claim to correct?

## The decision

**Correct the claim now, and name the fix.** The row is `partial`, and it says plainly that a
low-colour terminal still receives 256-colour indices.

## The reason

The audit proved three things, and each is true of the code today:

- `role_none` is **never called** outside its own tests. The no-colour mode exists as a table and as
  a test, and no code path selects it.
- `role_16` is called for exactly two fields, `bold` and `italic`. Its colour is never read.
- `RoleStyle::dim` and `RoleStyle::reversed` are set in both tables and **read by nothing**.

So the sentence was false in the part that mattered: the mappings exist, and nothing chooses between
them. rho emits a 256-colour index whatever the terminal reports.

This work made it worse rather than better. The table went from six roles to twelve, so there is now
twice as much unselected mapping behind the same claim.

## Why the claim moves and the code does not

Selecting a mode needs a decision that is not this feature's to make. It has to read the terminal's
capability, from `TERM`, `COLORTERM`, and the `NO_COLOR` convention, resolve it **once** at startup,
and thread the answer to `style_for`, which is a pure function called per row. That is a contract
change to the theme surface, and a contract change gets its own spec and its own review.

Shipping half of it, an environment read inside `style_for`, would be worse: a per-cell environment
lookup, and a pure function that is no longer pure.

## What is kept, and why

`RoleStyle::dim` and `RoleStyle::reversed` stay, unread. They are data and not logic, they are the
answer each role already gives for a mode that is coming, and deleting them would throw away the
part of the work that is right. The claim now says they are not selected, so no reader is misled.

## What this rules out

- **No claim that rho adapts to a terminal's colour depth.** It does not, until a spec says how.
- **No environment read inside `style_for`.** The mode resolves once, at startup, or not at all.
- **No new role without all three mappings.** The exhaustive matches force that, and
  `theme_resolves_every_role` keeps it honest even while two of the three are unused.
