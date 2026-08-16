/**
 * The card both authentication screens sit in.
 *
 * `.si-card` from `design/screens/sign-in.html` and `.as-card` from
 * `design/screens/auth-states.html` are the same card at two widths, one of
 * them centre-aligned. Neither is in `design/components.css` — they are
 * screen-local styles in their own design files — so they are reproduced here
 * as inline style rather than invented as new design-system classes.
 */

import type { ReactNode } from "react";

export function AuthCard({
  children,
  /** `.as-card` is 400px and centre-aligned; `.si-card` is 380px and not. */
  variant = "signin",
  labelledBy,
}: {
  children: ReactNode;
  variant?: "signin" | "state";
  labelledBy?: string;
}) {
  return (
    <section
      aria-labelledby={labelledBy}
      style={{
        width: "100%",
        maxWidth: variant === "state" ? "400px" : "380px",
        textAlign: variant === "state" ? "center" : undefined,
        background: "var(--pv-bg)",
        border: "1px solid var(--pv-border)",
        borderRadius: "var(--pv-radius-xl)",
        boxShadow: "var(--pv-shadow-2)",
        padding: "var(--pv-space-8) var(--pv-space-6)",
      }}
    >
      {children}
    </section>
  );
}
