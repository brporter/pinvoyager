import type { ReactNode } from "react";

/** Shared document layout and attribution for the adapted legal drafts. */
export function LegalPage({ title, children }: { title: string; children: ReactNode }) {
  return (
    <article className="pv-legal" aria-label={title}>
      <header>
        <p className="pv-legal-status">Draft for review · Updated October 18, 2026</p>
        <h1>{title}</h1>
        <p>
          Operator: Bryan Porter (Missouri, United States of America). Contact:{" "}
          <a href="mailto:inquiries@bryanporter.com">inquiries@bryanporter.com</a>. These drafts are
          pending review and do not yet have an effective date.
        </p>
      </header>
      {children}
      <aside className="pv-legal-attribution" aria-label="Template attribution">
        Adapted from the{" "}
        <a href="https://github.com/appdotnet/template-terms-of-service">App.net legal templates</a>
        , with changes for PinVoyager. This document is licensed under{" "}
        <a href="https://creativecommons.org/licenses/by-sa/3.0/">CC BY-SA 3.0</a>. This license
        covers the policy text, not users’ photos or the PinVoyager brand.
      </aside>
    </article>
  );
}
