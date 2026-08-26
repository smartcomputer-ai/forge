import { and, asc, eq, sql } from "drizzle-orm";
import { schema, type Db } from "@lightspeed/platform-db";
import type { ChannelProvider, NormalizedInboundV1 } from "../contracts/channel.js";
import type { ChannelRoute } from "../contracts/channel.js";
import { channelPairingKey } from "../identity/ids.js";
import {
  resolveActivationSettings,
  type ChannelActivationSettings,
} from "../policy/activation.js";
import {
  authorizeChannelSender,
  resolveAccessSettings,
  type ChannelAccessSettings,
  type ChannelAuthorization,
  type ChannelMemberRole,
} from "../policy/access.js";

/**
 * The control plane resolves a conversation to a bot's `chat` trigger. A
 * chat connection is a trigger row, never its own record: the trigger's
 * spec carries the account, scope, activation, access, and pairing gate; the
 * bot carries the profile; routing is the trigger's generic `route`.
 */

export interface ChatTriggerCandidate {
  triggerId: string;
  triggerName: string;
  botId: string;
  botName: string;
  botEnabled: boolean;
  channelAccountId: string;
  accountProvider: ChannelProvider;
  accountId: string;
  /** Lightspeed universe id. */
  universeId: string;
  universeName: string;
  universeActive: boolean;
  enabled: boolean;
  matchScope: "direct" | "group" | null;
  priority: number;
  pairingRequired: boolean;
  pairingCode: string | null;
  paired: boolean;
  activation: unknown;
  access: unknown;
  memberRole: ChannelMemberRole;
}

export interface ResolvedChatTrigger {
  triggerId: string;
  triggerName: string;
  botId: string;
  botName: string;
  universeId: string;
  universeName: string;
  activation: ChannelActivationSettings;
  access: ChannelAccessSettings;
  authorization: ChannelAuthorization;
}

export interface ChatTriggerResolver {
  resolve(inbound: NormalizedInboundV1): Promise<ResolvedChatTrigger | null>;
}

export type ChannelAdmissionDecision =
  | { status: "bound"; trigger: ResolvedChatTrigger }
  | { status: "paired"; trigger: ResolvedChatTrigger }
  | { status: "pairing_required" }
  | { status: "pairing_pending" }
  | { status: "unbound" };

export interface ChannelControlPlane extends ChatTriggerResolver {
  admit(inbound: NormalizedInboundV1): Promise<ChannelAdmissionDecision>;
  pairingRequired(route: ChannelRoute, scope: "direct" | "group"): Promise<boolean>;
}

export type ChannelAdmissionPlan =
  | { status: "bound"; trigger: ResolvedChatTrigger }
  | { status: "pair"; candidate: ChatTriggerCandidate }
  | { status: "pairing_required" }
  | { status: "pairing_pending" }
  | { status: "unbound" };

export function selectChatTrigger(
  candidates: readonly ChatTriggerCandidate[],
  route: ChannelRoute,
  isDirect: boolean,
): ResolvedChatTrigger | null {
  const candidate = matchingCandidates(candidates, route, isDirect).find(
    (row) => !row.pairingRequired || row.paired,
  );
  return candidate === undefined ? null : resolveCandidate(candidate, isDirect);
}

export function planChannelAdmission(
  candidates: readonly ChatTriggerCandidate[],
  inbound: NormalizedInboundV1,
): ChannelAdmissionPlan {
  const relevant = matchingCandidates(candidates, inbound.route, inbound.isDirect);
  const pairable: ChatTriggerCandidate[] = [];
  for (const candidate of relevant) {
    if (!candidate.pairingRequired) {
      if (pairable.length === 0) {
        return { status: "bound", trigger: resolveCandidate(candidate, inbound.isDirect) };
      }
      break;
    }
    pairable.push(candidate);
  }
  const alreadyPaired = pairable.find((candidate) => candidate.paired);
  if (alreadyPaired !== undefined) {
    return { status: "bound", trigger: resolveCandidate(alreadyPaired, inbound.isDirect) };
  }
  const code = inbound.text.trim();
  const matched =
    code.length === 0 ? undefined : pairable.find((candidate) => candidate.pairingCode === code);
  if (matched !== undefined) {
    return { status: "pair", candidate: matched };
  }
  if (pairable.length === 0) {
    return { status: "unbound" };
  }
  return shouldPromptForPairing(inbound, pairable)
    ? { status: "pairing_required" }
    : { status: "pairing_pending" };
}

export function createDbChannelControlPlane(db: Db): ChannelControlPlane {
  return {
    async resolve(inbound) {
      const candidates = await readCandidates(db, inbound);
      return selectChatTrigger(candidates, inbound.route, inbound.isDirect);
    },
    async admit(inbound) {
      const plan = planChannelAdmission(await readCandidates(db, inbound), inbound);
      if (plan.status !== "pair") {
        return plan;
      }
      await db
        .insert(schema.channelPairings)
        .values({
          key: channelPairingKey(inbound.route),
          triggerId: plan.candidate.triggerId,
          channelAccountId: plan.candidate.channelAccountId,
          chatId: inbound.route.chatId,
        })
        .onConflictDoUpdate({
          target: schema.channelPairings.key,
          set: {
            triggerId: plan.candidate.triggerId,
            channelAccountId: plan.candidate.channelAccountId,
            chatId: inbound.route.chatId,
            updatedAt: new Date(),
          },
        });
      return {
        status: "paired",
        trigger: resolveCandidate({ ...plan.candidate, paired: true }, inbound.isDirect),
      };
    },
    async pairingRequired(route, scope) {
      const inbound: NormalizedInboundV1 = {
        version: 1,
        messageId: "pairing-probe",
        route,
        senderId: "pairing-probe",
        senderName: "pairing-probe",
        timestampMs: 0,
        text: "",
        isDirect: scope === "direct",
        mentionedBot: false,
        isReplyToBot: false,
      };
      const plan = planChannelAdmission(await readCandidates(db, inbound), inbound);
      return plan.status === "pairing_pending" || plan.status === "pairing_required";
    },
  };
}

const chatAccountId = sql<string>`(${schema.botTriggers.spec}->>'channelAccountId')::uuid`;
const chatPriority = sql<number>`coalesce((${schema.botTriggers.spec}->>'priority')::int, 100)`;

async function readCandidates(
  db: Db,
  inbound: NormalizedInboundV1,
): Promise<ChatTriggerCandidate[]> {
  const route = inbound.route;
  const rows = await db
    .select({
      triggerId: schema.botTriggers.id,
      triggerName: schema.botTriggers.name,
      spec: schema.botTriggers.spec,
      enabled: schema.botTriggers.enabled,
      botId: schema.bots.id,
      botName: schema.bots.name,
      botEnabled: schema.bots.enabled,
      channelAccountId: schema.channelAccounts.id,
      accountProvider: schema.channelAccounts.provider,
      accountId: schema.channelAccounts.accountId,
      universeId: schema.universes.lightspeedUniverseId,
      universeName: schema.universes.name,
      universeStatus: schema.universes.status,
      pairingKey: schema.channelPairings.key,
      memberRole: schema.member.role,
    })
    .from(schema.botTriggers)
    .innerJoin(schema.bots, eq(schema.bots.id, schema.botTriggers.botId))
    .innerJoin(schema.universes, eq(schema.universes.id, schema.bots.universeId))
    .innerJoin(
      schema.channelAccounts,
      and(
        eq(schema.channelAccounts.id, chatAccountId),
        eq(schema.channelAccounts.provider, route.provider),
        eq(schema.channelAccounts.accountId, route.accountId),
        eq(schema.channelAccounts.enabled, true),
      ),
    )
    .leftJoin(
      schema.channelPairings,
      and(
        eq(schema.channelPairings.triggerId, schema.botTriggers.id),
        eq(schema.channelPairings.channelAccountId, schema.channelAccounts.id),
        eq(schema.channelPairings.chatId, route.chatId),
      ),
    )
    .leftJoin(
      schema.channelIdentities,
      and(
        eq(schema.channelIdentities.channel, route.provider),
        eq(schema.channelIdentities.handle, inbound.senderId),
      ),
    )
    .leftJoin(
      schema.member,
      and(
        eq(schema.member.userId, schema.channelIdentities.userId),
        eq(schema.member.organizationId, schema.universes.organizationId),
      ),
    )
    .where(and(eq(schema.botTriggers.kind, "chat"), eq(schema.botTriggers.enabled, true)))
    .orderBy(asc(chatPriority), asc(schema.botTriggers.createdAt));
  return rows.map((row) => {
    const spec = row.spec as {
      matchScope?: "direct" | "group" | null;
      priority?: number;
      pairingCode?: string | null;
      activation?: unknown;
      access?: unknown;
    };
    return {
      triggerId: row.triggerId,
      triggerName: row.triggerName,
      botId: row.botId,
      botName: row.botName,
      botEnabled: row.botEnabled,
      channelAccountId: row.channelAccountId,
      accountProvider: row.accountProvider,
      accountId: row.accountId,
      universeId: row.universeId,
      universeName: row.universeName,
      universeActive: row.universeStatus === "active",
      enabled: row.enabled,
      matchScope: spec.matchScope ?? null,
      priority: spec.priority ?? 100,
      pairingRequired: spec.pairingCode != null,
      pairingCode: spec.pairingCode ?? null,
      paired: row.pairingKey !== null,
      activation: spec.activation ?? null,
      access: spec.access ?? null,
      memberRole: memberRole(row.memberRole),
    };
  });
}

function matchingCandidates(
  candidates: readonly ChatTriggerCandidate[],
  route: ChannelRoute,
  isDirect: boolean,
): ChatTriggerCandidate[] {
  const scope = isDirect ? "direct" : "group";
  return candidates
    .filter(
      (row) =>
        row.enabled &&
        row.botEnabled &&
        row.universeActive &&
        row.accountProvider === route.provider &&
        row.accountId === route.accountId &&
        (row.matchScope === null || row.matchScope === scope),
    )
    .sort((a, b) => a.priority - b.priority);
}

function resolveCandidate(
  candidate: ChatTriggerCandidate,
  isDirect: boolean,
): ResolvedChatTrigger {
  const scope = isDirect ? "direct" : "group";
  const access = resolveAccessSettings(candidate.access);
  return {
    triggerId: candidate.triggerId,
    triggerName: candidate.triggerName,
    botId: candidate.botId,
    botName: candidate.botName,
    universeId: candidate.universeId,
    universeName: candidate.universeName,
    activation: resolveActivationSettings(scope, candidate.activation),
    access,
    authorization: authorizeChannelSender(access, candidate.memberRole),
  };
}

function shouldPromptForPairing(
  inbound: NormalizedInboundV1,
  candidates: readonly ChatTriggerCandidate[],
): boolean {
  if (inbound.isDirect || inbound.mentionedBot || inbound.isReplyToBot) {
    return true;
  }
  const text = inbound.text.trim().toLowerCase();
  return candidates.some((candidate) =>
    resolveActivationSettings("group", candidate.activation).triggerPrefixes.some((prefix) => {
      const normalized = prefix.toLowerCase();
      return text === normalized || text.startsWith(`${normalized} `) || text.startsWith(`${normalized}@`);
    }),
  );
}

function memberRole(value: string | null): ChannelMemberRole {
  return value === "member" || value === "admin" || value === "owner" ? value : null;
}
