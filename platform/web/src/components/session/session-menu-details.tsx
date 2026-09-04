import {
  DropdownMenuGroup,
  DropdownMenuLabel,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu";

export function sessionMenuMetadataEntries(
  metadata: Record<string, string> | undefined,
): Array<[string, string]> {
  return Object.entries(metadata ?? {}).sort(([left], [right]) =>
    left.localeCompare(right),
  );
}

/** Compact, read-only session identity shared by the bot and Sessions menus. */
export function SessionMenuIdentity({ sessionId }: { sessionId: string }) {
  return (
    <DropdownMenuGroup>
      <DropdownMenuLabel>Session</DropdownMenuLabel>
      <div className="min-w-0 px-2 pb-1.5">
        <code
          className="block min-w-0 truncate text-xs text-foreground"
          title={sessionId}
        >
          {sessionId}
        </code>
      </div>
    </DropdownMenuGroup>
  );
}

/** Optional metadata appendix; includes its own separator for last position. */
export function SessionMenuMetadata({ metadata }: { metadata?: Record<string, string> }) {
  const entries = sessionMenuMetadataEntries(metadata);
  if (entries.length === 0) return null;
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuLabel>Metadata</DropdownMenuLabel>
        <div className="grid min-w-0 gap-1 px-2 pb-1.5">
          {entries.map(([key, value]) => (
            <div
              key={key}
              className="grid min-w-0 grid-cols-[minmax(0,2fr)_minmax(0,3fr)] items-baseline gap-2 text-xs"
            >
              <code className="min-w-0 truncate text-muted-foreground" title={key}>
                {key}
              </code>
              <span className="min-w-0 truncate text-foreground" title={value}>
                {value}
              </span>
            </div>
          ))}
        </div>
      </DropdownMenuGroup>
    </>
  );
}
