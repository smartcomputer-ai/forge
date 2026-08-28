/// Platform-wide fixtures: users beside the demo admin, operator-registered
/// environment providers, channel accounts, and connector health.
import type { DemoStore, DemoUser } from "../store";

export const INCUS_PROVIDER_ID = "incus-eu-1";
export const TELEGRAM_ACCOUNT_ID = "chan-telegram-northwind";
export const WHATSAPP_ACCOUNT_ID = "chan-whatsapp-northwind";

const OTHER_USERS: DemoUser[] = [
  {
    id: "user-marco",
    name: "Marco Ruiz",
    email: "marco@acme.example",
    role: "user",
    emailVerified: true,
    image: null,
    banned: false,
    createdAt: "2026-06-03T10:00:00.000Z",
    updatedAt: "2026-08-01T08:30:00.000Z",
  },
  {
    id: "user-priya",
    name: "Priya Natarajan",
    email: "priya@acme.example",
    role: "user",
    emailVerified: true,
    image: null,
    banned: false,
    createdAt: "2026-06-10T15:45:00.000Z",
    updatedAt: "2026-08-12T11:20:00.000Z",
  },
  {
    id: "user-jonas",
    name: "Jonas Lindqvist",
    email: "jonas@northwind.example",
    role: "user",
    emailVerified: true,
    image: null,
    banned: false,
    createdAt: "2026-07-01T09:00:00.000Z",
    updatedAt: "2026-08-18T16:40:00.000Z",
  },
];

export function seedPlatform(store: DemoStore): void {
  for (const user of OTHER_USERS) store.users.set(user.id, { ...user });

  store.environmentProviders.set(INCUS_PROVIDER_ID, {
    providerId: INCUS_PROVIDER_ID,
    displayName: "Incus (eu-1)",
    controllerConnection: {
      endpoint: "wss://incus-eu-1.lightspeed.demo:19090",
      transport: { type: "webSocket" },
    },
    metadata: { region: "eu-1", mode: "cluster" },
    createdAtMs: Date.parse("2026-06-05T12:00:00.000Z"),
    updatedAtMs: Date.parse("2026-08-10T09:00:00.000Z"),
  });

  store.channelAccounts.set(TELEGRAM_ACCOUNT_ID, {
    id: TELEGRAM_ACCOUNT_ID,
    provider: "telegram",
    accountId: "northwind_support_bot",
    displayName: "Northwind Developer Support (Telegram)",
    settings: {},
    enabled: true,
    createdAt: "2026-07-02T10:00:00.000Z",
    updatedAt: "2026-08-15T10:00:00.000Z",
  });
  store.channelAccounts.set(WHATSAPP_ACCOUNT_ID, {
    id: WHATSAPP_ACCOUNT_ID,
    provider: "whatsapp",
    accountId: "+4915112345678",
    displayName: "Northwind Developer Support (WhatsApp)",
    settings: { printQr: false },
    enabled: true,
    createdAt: "2026-07-20T10:00:00.000Z",
    updatedAt: "2026-08-22T10:00:00.000Z",
  });

  const changedAtMs = Date.now() - 42 * 60_000;
  store.channelsStatus = {
    connectors: [
      {
        url: "http://channels-telegram.internal:9101/health",
        reachable: true,
        httpStatus: 200,
        health: {
          version: 1,
          provider: "telegram",
          accountId: "northwind_support_bot",
          state: "ready",
          ingressConnected: true,
          activityWorkerReady: true,
          reconnectAttempts: 0,
          changedAtMs,
        },
      },
      {
        url: "http://channels-whatsapp.internal:9102/health",
        reachable: true,
        httpStatus: 200,
        health: {
          version: 1,
          provider: "whatsapp",
          accountId: "+4915112345678",
          state: "disconnected",
          ingressConnected: false,
          activityWorkerReady: true,
          reconnectAttempts: 3,
          detail: "waiting for the phone to come back online",
          lastError: "websocket closed (1006)",
          lastErrorAtMs: changedAtMs,
          changedAtMs,
        },
      },
    ],
  };
}
