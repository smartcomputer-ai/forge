/// Always-visible warning so nobody mistakes the demo for a deployment.
/// Sits in the sidebar's empty space above the user menu, where it covers
/// no page title, header action, or composer; dismissible for screenshots.
export function mountBanner(): void {
  const banner = document.createElement("div");
  banner.setAttribute("role", "status");
  banner.className =
    "fixed bottom-20 left-3 z-50 max-w-[232px] rounded-md border border-amber-600/60 bg-amber-100 px-3 py-2 text-xs leading-snug text-amber-950 shadow-md dark:border-amber-500/50 dark:bg-amber-950 dark:text-amber-100";

  const heading = document.createElement("div");
  heading.className = "flex items-center gap-1.5 font-semibold";
  heading.innerHTML =
    '<svg aria-hidden="true" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round" stroke-linejoin="round"><path d="M12 3 2.5 20h19L12 3Z"/><path d="M12 9v5"/><path d="M12 17.5h.01"/></svg><span>Demo mode</span>';

  const text = document.createElement("p");
  text.className = "mt-1";
  text.textContent = "Runs in your browser on scripted data. Nothing is saved or sent anywhere.";

  const actions = document.createElement("div");
  actions.className = "mt-1.5 flex gap-3";
  const reset = document.createElement("button");
  reset.type = "button";
  reset.textContent = "Reset";
  reset.className = "font-semibold underline underline-offset-2 hover:opacity-80";
  reset.addEventListener("click", () => {
    window.location.assign("/app/");
  });
  const dismiss = document.createElement("button");
  dismiss.type = "button";
  dismiss.textContent = "Hide";
  dismiss.className = "underline underline-offset-2 hover:opacity-80";
  dismiss.addEventListener("click", () => banner.remove());
  actions.append(reset, dismiss);

  banner.append(heading, text, actions);
  document.body.append(banner);
}
