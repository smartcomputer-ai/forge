// @vitest-environment jsdom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { TranscriptEntryView, UserBand } from "./transcript-view";

let root: Root;
let container: HTMLDivElement;
let height: number;
let resize: () => void;
const disconnect = vi.fn();

beforeEach(() => {
  height = 40;
  disconnect.mockClear();
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.stubGlobal("ResizeObserver", class {
    constructor(callback: () => void) { resize = callback; }
    observe() {}
    disconnect = disconnect;
  });
  vi.spyOn(HTMLElement.prototype, "scrollHeight", "get").mockImplementation(() => height);
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

it.each([
  ["user:operator", "default"], ["event", "muted"], ["integration:example", "muted"],
  [undefined, "muted"], ["user:", "muted"],
])("uses the appropriate palette for origin %s without adding a label", async (origin, variant) => {
  await act(async () => root.render(<TranscriptEntryView entry={{ kind: "message", key: "message", role: "user", text: "Hello", origin }} />));
  expect(container.querySelector('[data-slot="bubble"]')?.getAttribute("data-variant")).toBe(variant);
  expect(container.textContent).toBe("Hello");
  expect(container.querySelector("button")).toBeNull();
});

it.each([true, false])("collapses and expands overflowing messages, human=%s", async (human) => {
  height = 700;
  const text = "Long input\n".repeat(50) + "Last line";
  await act(async () => root.render(<UserBand text={text} human={human} steering />));
  const button = container.querySelector("button")!;
  const content = document.getElementById(button.getAttribute("aria-controls")!)!;
  expect(button.textContent).toBe("Show more");
  expect(button.getAttribute("aria-expanded")).toBe("false");
  expect(content.style.maxHeight).toBe("160px");
  expect(content.style.maskImage).toContain("linear-gradient");
  expect(content.textContent).toContain("Last line");
  await act(async () => button.click());
  expect(button.textContent).toBe("Show less");
  expect(button.getAttribute("aria-expanded")).toBe("true");
  expect(content.style.maxHeight).toBe("");
  expect(content.style.maskImage).toBe("");
  await act(async () => button.click());
  expect(content.style.maxHeight).toBe("160px");
});

it("rechecks overflow on resize and text changes, and disconnects on unmount", async () => {
  await act(async () => root.render(<UserBand text="A message" />));
  expect(container.querySelector("button")).toBeNull();
  height = 400;
  await act(async () => resize());
  expect(container.querySelector("button")?.textContent).toBe("Show more");
  height = 20;
  await act(async () => root.render(<UserBand text="Shorter" />));
  expect(container.querySelector("button")).toBeNull();
  expect(disconnect).toHaveBeenCalledOnce();
  await act(async () => root.render(null));
  expect(disconnect).toHaveBeenCalledTimes(2);
});

it("uses the human palette immediately for an optimistic message", async () => {
  await act(async () => root.render(<UserBand text="Sending" human pending />));
  expect(container.querySelector('[data-slot="bubble"]')?.getAttribute("data-variant")).toBe("default");
});
