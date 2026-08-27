/// A bot id is derived from its name until edited, then it is the person's.
export function botIdFrom(displayName: string): string {
  return displayName
    .toLowerCase()
    .normalize("NFKD")
    .replace(/[\u0300-\u036f]/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

/// "Reviewer · reviewer" reads as a stutter: the id is worth showing next
/// to the name only when it says something the name does not.
export function idIsRedundant(displayName: string | null, botId: string): boolean {
  return displayName !== null && botIdFrom(displayName) === botId;
}
