/**
 * "Skip to content", the first thing in the tab order.
 *
 * The design system provides `.pv-sr`, which hides an element from sight
 * permanently — correct for a label, wrong for a skip link, which has to become
 * visible when it receives focus or it is a control a keyboard user can reach
 * and cannot see. The usual fix is a `:focus` rule in CSS; this phase writes no
 * CSS, so the same effect is produced by swapping the class on focus.
 */

import { useState } from "react";

/** @param targetId - id of the `<main>` this jumps to. */
export function SkipLink({ targetId }: { targetId: string }) {
  const [focused, setFocused] = useState(false);

  return (
    <a
      className={focused ? undefined : "pv-sr"}
      href={`#${targetId}`}
      onFocus={() => {
        setFocused(true);
      }}
      onBlur={() => {
        setFocused(false);
      }}
      style={
        focused
          ? {
              position: "absolute",
              zIndex: 100,
              top: "var(--pv-space-2)",
              left: "var(--pv-space-2)",
              display: "inline-flex",
              alignItems: "center",
              minHeight: "var(--pv-touch-min)",
              padding: "0 var(--pv-space-4)",
              borderRadius: "var(--pv-radius-md)",
              background: "var(--pv-bg-raised)",
              color: "var(--pv-text)",
              boxShadow: "var(--pv-shadow-2)",
              textDecoration: "none",
              fontSize: "var(--pv-text-sm)",
            }
          : undefined
      }
    >
      Skip to content
    </a>
  );
}
