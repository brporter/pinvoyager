/** TextField uses the shared runtime styles and accessible native controls. */

import { useId } from "react";
import type { InputHTMLAttributes } from "react";

import { FieldShell } from "./Field";
import { fieldIds } from "./fieldIds";
import type { FieldChrome } from "./Field";

export interface TextFieldProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "className">, FieldChrome {}

export function TextField({ label, help, error, required, id, ...rest }: TextFieldProps) {
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
