// Lightspeed's long-lived default universe uses the canonical UUID text shape
// with a zero version nibble (`00000000-0000-0000-0000-000000000001`).
// Universe ids are opaque tenant identifiers here, so validate their shape
// without imposing RFC version/variant bits that Lightspeed itself does not.
const UUID = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

/** One served account: an account id inside its universe. */
export interface AccountSelector {
  universeId: string;
  accountId: string;
}

/** The host's map key for an account: `<universeId>/<accountId>`. */
export function accountKey(universeId: string, accountId: string): string {
  return `${normalizeUniverseId(universeId)}/${requirePart(accountId, "accountId")}`;
}

/** Parse a `LIGHTSPEED_CONNECTOR_ACCOUNTS` entry (`<universeId>/<accountId>`). */
export function parseAccountSelector(value: string): AccountSelector {
  const separator = value.indexOf("/");
  if (separator <= 0 || separator === value.length - 1) {
    throw new TypeError(
      `invalid account selector ${JSON.stringify(value)}; expected <universeId>/<accountId>`,
    );
  }
  return {
    universeId: normalizeUniverseId(value.slice(0, separator)),
    accountId: requirePart(value.slice(separator + 1), "accountId"),
  };
}

export function normalizeUniverseId(value: string): string {
  if (!UUID.test(value)) {
    throw new TypeError("universeId must be a UUID");
  }
  return value.toLowerCase();
}

function requirePart(value: string, name: string): string {
  if (value.length === 0) {
    throw new TypeError(`${name} must not be empty`);
  }
  return value;
}
