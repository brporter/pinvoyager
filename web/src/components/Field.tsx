/**
 * The shared scaffolding behind TextField, TextArea and Select — the label,
 * help text and error row drawn in `design/components/forms.html`.
 *
 * The interesting part is the error wiring. An error is three things at once:
 *
 *  1. `aria-invalid="true"` on the control, which the design system turns into
 *     a red border.
 *  2. A visible message carrying an icon as well as colour, because colour
 *     alone is never the only signal.
 *  3. `aria-describedby` pointing at that message, so a screen reader announces
 *     it when the control takes focus rather than leaving someone to discover a
 *     validation failure by trial.
 *
 * (3) is the one that gets forgotten, so it is done here once rather than at
 * every call site.
 */

import type { ReactNode } from "react";

import { AlertCircleIcon } from "./icons";

export interface FieldChrome {
  /** Visible label. Always present — a placeholder is not a label. */
  label: ReactNode;
  /** Marks the control required and shows the asterisk from the design. */
  required?: boolean;
  /** Guidance shown when there is no error. */
  help?: ReactNode;
  /** Validation message. Its presence is what puts the field in the error
   *  state; there is no separate `invalid` flag to keep in step with it. */
  error?: ReactNode;
}

export interface FieldIds {
  controlId: string;
  /** Value for the control's `aria-describedby`, or undefined when there is
   *  nothing to describe it with. */
  describedBy: string | undefined;
  helpId: string;
  errorId: string;
}

export function fieldIds(baseId: string, chrome: FieldChrome): FieldIds {
  const helpId = `${baseId}-help`;
  const errorId = `${baseId}-error`;
  const described = [chrome.error != null && errorId, chrome.help != null && helpId]
    .filter((value): value is string => typeof value === "string")
    .join(" ");

  return {
    controlId: baseId,
    describedBy: described === "" ? undefined : described,
    helpId,
    errorId,
  };
}

export function FieldShell({
  chrome,
  ids,
  children,
}: {
  chrome: FieldChrome;
  ids: FieldIds;
  children: ReactNode;
}) {
  return (
    <div className="pv-field">
      <label className="pv-field-label" htmlFor={ids.controlId}>
        {chrome.label}
        {chrome.required === true && (
          // Hidden from assistive technology: the control carries `required`,
          // so announcing the asterisk as well would say it twice.
          <span className="pv-field-req" aria-hidden="true">
            *
          </span>
        )}
      </label>
      {children}
      {chrome.error != null && (
        <span className="pv-field-error" id={ids.errorId}>
          <AlertCircleIcon size={12} strokeWidth={2.5} />
          {chrome.error}
        </span>
      )}
      {chrome.help != null && (
        <span className="pv-field-help" id={ids.helpId}>
          {chrome.help}
        </span>
      )}
    </div>
  );
}
