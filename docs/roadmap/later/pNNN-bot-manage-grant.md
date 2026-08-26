# Later — Bot `manage` grant (cross-bot authority)

Recorded from [P135](../p135-bot-federation.md) so the decision can be
revisited with evidence rather than re-derived. P135's adopted position is
**no cross-bot authority**: bot ↔ bot is events only, a bot reshapes itself
within `selfConfig`, humans set every bot's scope. This note is the
alternative to reach for if a real use case demands authority — an ops bot
that must act on neighbours without waiting for their briefs to agree, or
must stand up bots on demand.

Configuration and creation are the same question (a poll trigger put on a
neighbour is as durable and as costly as a new bot), so the grant covers
both at once:

```text
manage: {
  bots: string[],                             // names this bot may configure
  ops: ("trigger" | "brief" | "enable")[],
  create?: { profiles: string[], maxBots: number }
} | null
```

- Configure: the `selfConfig` tools grow an optional `bot?` target;
  `bot_enable { bot, enabled }` is target-only (a bot never un-pauses
  itself); both feeds record `managed` / `configured_by`; the target's
  controller gets the config signal as after a UI edit.
- Create: `bot_create { name, profileId, brief?, runsPerDay?,
  acceptsBotEvents? }` with `profileId` ∈ `create.profiles` (the profile is
  the capability container, so the allowlist is the authority — the
  `features.subagents.agents[]` idea one tier up); attenuation fixed at one
  level (created bots get `selfConfig` / `emit` only if the creator has
  them, `runsPerDay` ≤ the creator's, `manage: null` always); `maxBots`
  counts *live* bots whose origin is the creator, because a bot is standing
  cost; `bots.origin = { botId, botName, seq, sessionId, runId }` as
  provenance never ownership (deleting the creator nulls it); management
  authority = `manage.bots ∪ { bots whose origin is me }`; deletion stays
  human.

Build it whole or not at all. Half of it — configuration without creation,
or the reverse — has no principled boundary.
