/** Connects a control to its label, hint, required indicator, and validation message. */

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
