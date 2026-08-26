import { useEffect, useState } from "react";

export type PresenceState = "open" | "closed";

export function usePresence(open: boolean, exitDuration = 220) {
  const [mounted, setMounted] = useState(open);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    let frame = 0;
    let timer = 0;

    if (open) {
      setMounted(true);
      frame = window.requestAnimationFrame(() => setVisible(true));
    } else {
      setVisible(false);
      if (mounted) {
        timer = window.setTimeout(() => setMounted(false), exitDuration);
      }
    }

    return () => {
      if (frame) window.cancelAnimationFrame(frame);
      if (timer) window.clearTimeout(timer);
    };
  }, [exitDuration, mounted, open]);

  return {
    mounted,
    state: (visible ? "open" : "closed") as PresenceState,
  };
}
