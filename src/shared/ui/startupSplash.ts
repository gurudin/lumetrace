export const startupSplashId = "lume-trace-startup";

const startupSplashTransitionFallbackMs = 220;

export function dismissStartupSplash() {
  if (typeof document === "undefined" || typeof window === "undefined") return () => undefined;
  const splash = document.getElementById(startupSplashId);
  if (!splash) return () => undefined;

  if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) {
    splash.remove();
    return () => undefined;
  }

  let timeoutId: number | null = null;
  const removeSplash = () => {
    if (timeoutId !== null) window.clearTimeout(timeoutId);
    timeoutId = null;
    splash.removeEventListener("transitionend", removeSplash);
    splash.remove();
  };
  const frameId = window.requestAnimationFrame(() => {
    splash.dataset.state = "closing";
    splash.setAttribute("aria-busy", "false");
    splash.addEventListener("transitionend", removeSplash, { once: true });
    timeoutId = window.setTimeout(removeSplash, startupSplashTransitionFallbackMs);
  });

  return () => {
    window.cancelAnimationFrame(frameId);
    if (timeoutId !== null) window.clearTimeout(timeoutId);
    splash.removeEventListener("transitionend", removeSplash);
  };
}
