import { Command } from "commander";
import { api, printJson } from "./client.js";
import { loadConfig, saveConfig } from "./config.js";
import { promptHidden } from "./prompt.js";

const program = new Command("lightspeed-platform")
  .description("Lightspeed platform administration CLI")
  .configureHelp({ sortSubcommands: true });

program
  .command("login")
  .description("authenticate against the platform and store a bearer token")
  .requiredOption("--email <email>")
  .option("--url <url>", "platform base URL (persisted)")
  .option("--password <password>", "password (prompted when omitted)")
  .action(async (opts: { email: string; url?: string; password?: string }) => {
    const config = loadConfig();
    const baseUrl = opts.url ?? config.baseUrl;
    const password = opts.password ?? (await promptHidden("Password: "));
    const res = await fetch(new URL("/api/auth/sign-in/email", baseUrl), {
      method: "POST",
      headers: {
        "content-type": "application/json",
        origin: new URL(baseUrl).origin,
      },
      body: JSON.stringify({ email: opts.email, password }),
    });
    const body = (await res.json()) as { token?: string };
    if (!res.ok) {
      console.error("Login failed:", JSON.stringify(body));
      process.exit(1);
    }
    // Bearer plugin surfaces the session token in the set-auth-token
    // header; recent versions also include it in the body.
    const token = res.headers.get("set-auth-token") ?? body.token;
    if (!token) {
      console.error("Login succeeded but no bearer token was returned.");
      process.exit(1);
    }
    saveConfig({ baseUrl, token });
    console.log(`Logged in to ${baseUrl}`);
  });

program
  .command("whoami")
  .description("show the authenticated user")
  .action(async () => printJson(await api("GET", "/api/v1/me")));

const universe = program.command("universe").description("manage universes");

universe
  .command("list")
  .action(async () => printJson(await api("GET", "/api/v1/universes")));

universe
  .command("create <name>")
  .option("--slug <slug>")
  .action(async (name: string, opts: { slug?: string }) =>
    printJson(
      await api("POST", "/api/v1/universes", {
        name,
        slug: opts.slug,
      }),
    ),
  );

universe
  .command("show <id>")
  .action(async (id: string) => printJson(await api("GET", `/api/v1/universes/${id}`)));

universe
  .command("archive <id>")
  .action(async (id: string) =>
    printJson(await api("PATCH", `/api/v1/universes/${id}`, { status: "archived" })),
  );

const member = program.command("member").description("manage universe members");

member
  .command("list")
  .requiredOption("--universe <id>")
  .action(async (opts: { universe: string }) =>
    printJson(await api("GET", `/api/v1/universes/${opts.universe}/members`)),
  );

member
  .command("add")
  .requiredOption("--universe <id>")
  .requiredOption("--user <userId>")
  .option("--role <role>", "owner | admin | member", "member")
  .action(async (opts: { universe: string; user: string; role: string }) =>
    printJson(
      await api("POST", `/api/v1/universes/${opts.universe}/members`, {
        userId: opts.user,
        role: opts.role,
      }),
    ),
  );

member
  .command("remove")
  .requiredOption("--universe <id>")
  .requiredOption("--member <memberId>")
  .action(async (opts: { universe: string; member: string }) =>
    printJson(
      await api(
        "DELETE",
        `/api/v1/universes/${opts.universe}/members/${opts.member}`,
      ),
    ),
  );

const channelAccount = program
  .command("channel-account")
  .description("manage Telegram and WhatsApp accounts");

channelAccount
  .command("list")
  .action(async () => printJson(await api("GET", "/api/v1/channel-accounts")));

channelAccount
  .command("add")
  .requiredOption("--provider <provider>", "telegram | whatsapp")
  .requiredOption("--account-id <id>", "stable connector account id")
  .requiredOption("--display-name <name>")
  .option("--credential-ref <ref>", "opaque secret-store reference")
  .option("--state-ref <ref>", "opaque provider-state reference")
  .action(
    async (opts: {
      provider: string;
      accountId: string;
      displayName: string;
      credentialRef?: string;
      stateRef?: string;
    }) =>
      printJson(
        await api("POST", "/api/v1/channel-accounts", {
          provider: opts.provider,
          accountId: opts.accountId,
          displayName: opts.displayName,
          credentialRef: opts.credentialRef ?? null,
          stateRef: opts.stateRef ?? null,
          settings: {},
        }),
      ),
  );

channelAccount
  .command("enable <id>")
  .action(async (id: string) =>
    printJson(await api("PATCH", `/api/v1/channel-accounts/${id}`, { enabled: true })),
  );

channelAccount
  .command("disable <id>")
  .action(async (id: string) =>
    printJson(await api("PATCH", `/api/v1/channel-accounts/${id}`, { enabled: false })),
  );

channelAccount
  .command("rm <id>")
  .action(async (id: string) =>
    printJson(await api("DELETE", `/api/v1/channel-accounts/${id}`)),
  );

const userCmd = program.command("user").description("manage platform users (admin)");

userCmd
  .command("create")
  .requiredOption("--email <email>")
  .requiredOption("--name <name>")
  .option("--role <role>", "user | admin", "user")
  .option("--password <password>", "prompted when omitted")
  .action(
    async (opts: { email: string; name: string; role: string; password?: string }) => {
      const password = opts.password ?? (await promptHidden("New user password: "));
      // better-auth admin plugin endpoint; bearer token must belong to a
      // platform admin.
      printJson(
        await api("POST", "/api/auth/admin/create-user", {
          email: opts.email,
          name: opts.name,
          password,
          role: opts.role,
        }),
      );
    },
  );

userCmd
  .command("list")
  .action(async () =>
    printJson(
      await api("GET", "/api/auth/admin/list-users?limit=100"),
    ),
  );

try {
  await program.parseAsync();
} catch (error) {
  console.error(error instanceof Error ? error.message : String(error));
  process.exit(1);
}
