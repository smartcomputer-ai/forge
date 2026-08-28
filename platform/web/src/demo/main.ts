/// Demo entry: install the in-browser backend, then load the real app.
/// `src/main.tsx` and everything under it are untouched by demo mode.
import { mountBanner } from "./banner";
import { installDemoFetch } from "./fetch";
import { createDemoStore } from "./fixtures";
import { createDemoRouter } from "./router";

installDemoFetch(createDemoRouter(createDemoStore()));
mountBanner();
await import("../main");
