import {
  DropdownMenuCheckboxItem, DropdownMenuGroup, DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu";
import { useUserPreferences } from "@/lib/user-preferences";

/** Shared account-wide display controls in session and bot conversation menus. */
export function SessionMenuPreferences() {
  const preferences = useUserPreferences();
  return (
    <>
      <DropdownMenuSeparator />
      <DropdownMenuGroup>
        <DropdownMenuCheckboxItem
          checked={preferences.showRunStatistics}
          onCheckedChange={preferences.setShowRunStatistics}
          closeOnClick={false}
        >
          Show run statistics
        </DropdownMenuCheckboxItem>
      </DropdownMenuGroup>
    </>
  );
}
