/**
 * TextField — the single-line input from `design/components/forms.html`.
 *
 * `.pv-input` sets a 16px font size and that is not a style choice: iOS Safari
 * zooms the viewport whenever a focused input is smaller than 16px, and the
 * zoom does not reverse cleanly, so the user is left with a horizontally
 * shifted layout mid-form. The label can be 12px; the input cannot. Do not
 * override the font size here.
 */

import { useId } from "react";
import type { InputHTMLAttributes } from "react";

import { FieldShell, fieldIds } from "./Field";
import type { FieldChrome } from "./Field";

export interface TextFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "className">,
    FieldChrome {}

export function TextField({
  label,
  help,
  error,
  required,
  id,
  ...rest
}: TextFieldProps) {
  const generatedId = useId();
  const chrome: FieldChrome = { label, help, error, required };
  const ids = fieldIds(id ?? generatedId, chrome);

  return (
    <FieldShell chrome={chrome} ids={ids}>
      <input
        {...rest}
        id={ids.controlId}
        className="pv-input"
        required={required}
        aria-invalid={error != null ? true : undefined}
        aria-describedby={ids.describedBy}
      />
    </FieldShell>
  );
}
