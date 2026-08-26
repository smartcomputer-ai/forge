# Later — Bots beyond federation

What else "make bots really good" means, apart from
[P135](../p135-bot-federation.md), ordered by how often it has already
bitten. Moved out of P135 so that document keeps to its one decision.

1. **`bot_trigger_put` raw secrets** — P133 removes the field; until then
   webhook/poll secrets transit CAS and history.
2. **Tier-2 per-trigger CEL projections** — the generic renderer covers
   current needs; revisit when a preset is not enough.
3. **Declaration rotation cost** — every grant flip or tool revision
   rotates the main session. In-place add-only declaration admission in
   core is the one core change worth its price, still "decide after v1
   contact".
4. **Email trigger, more presets, Channels bridge** — P131 ws2/4/5, parked
   or deferred by decision.
5. **Push transport for the UI** — long-poll everywhere; fine until it is
   not.
6. **Triage stage** — cheap model-side wake-vs-archive for ambient sources;
   still out until deterministic filters demonstrably fall short.

Done since the list was written: descendant-aware bot budgets (P134 counts
descendants through `session/list { rootSessionId }`).
