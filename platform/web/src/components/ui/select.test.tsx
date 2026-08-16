import { describe, expect, it } from "vitest";
import { renderToString } from "react-dom/server";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "./select";

/// The visible trigger label (the hidden form input legitimately carries the
/// raw value).
const triggerText = (node: React.ReactElement) =>
  renderToString(node).match(/data-slot="select-value"[^>]*>([^<]*)</)?.[1] ?? "";

describe("Select value label", () => {
  it("shows the selected item's label in the closed trigger, not its raw value", () => {
    const templates = [
      { key: "incus-dev:dev-small-v1", name: "Development VM (small)" },
      { key: "incus-dev:dev-large-v1", name: "Development VM (large)" },
    ];
    const label = triggerText(
      <Select value="incus-dev:dev-large-v1">
        <SelectTrigger><SelectValue /></SelectTrigger>
        <SelectContent>
          {templates.map((template) => (
            <SelectItem key={template.key} value={template.key}>{template.name}</SelectItem>
          ))}
        </SelectContent>
      </Select>,
    );
    expect(label).toBe("Development VM (large)");
  });

  it("keeps a SelectValue render function authoritative", () => {
    const label = triggerText(
      <Select value="b">
        <SelectTrigger>
          <SelectValue>{(value: string) => `custom ${value}`}</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="a">Alpha</SelectItem>
          <SelectItem value="b">Beta</SelectItem>
        </SelectContent>
      </Select>,
    );
    expect(label).toBe("custom b");
  });

  it("finds items nested in fragments and static lists", () => {
    const label = triggerText(
      <Select value="readOnly">
        <SelectTrigger><SelectValue /></SelectTrigger>
        <SelectContent>
          <>
            <SelectItem value="none">No file tools</SelectItem>
            <SelectItem value="readOnly">Read only</SelectItem>
          </>
          <SelectItem value="edit">Edit files</SelectItem>
        </SelectContent>
      </Select>,
    );
    expect(label).toBe("Read only");
  });
});
