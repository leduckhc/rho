# D-a-listing-failure-never-stops-a-session

Date: 20260831-201551

## The question

Listing is a network call. What happens when it fails, or when it has never run? The MCP
schema cache showed an empty first run, and that confused a first-time user.

## The decision

- rho lists lazily, on the first `/model` open, not at startup.
- The picker is never empty. It shows the current model at once, before the call returns.
- A listing failure keeps the session and the current model alive. The picker shows one line
  naming the failure, and still lets the user type an id.
- rho caches the list at `~/.rho/model-catalog-cache.json`, keyed by a fingerprint of the
  provider id and its endpoint. An entry lives for 24 hours. A stale entry shows, marked stale.
- A typed id always runs, even when the list omits it. The list is advisory.

## What this rules out

- Listing at startup, which would slow every session, even one that never opens the picker.
- An empty picker on a first run, which the MCP cache proved confusing.
- Rejecting a typed id because the list omits it. `models.md` shows a needed id form can be
  absent from a listing.

## Why

A session that already has a working model must not depend on a slow or failing list call.
The cache follows the MCP precedent, and the never-empty picker fixes the MCP first-run defect.
