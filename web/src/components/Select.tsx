/** Select uses the shared runtime styles and accessible native controls. */

import { useId } from "react";
import type { ReactNode, SelectHTMLAttributes } from "react";

import { FieldShell } from "./Field";
import { fieldIds } from "./fieldIds";
import type { FieldChrome } from "./Field";

export interface SelectProps
  extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "className">, FieldChrome {
  children: ReactNode;
}

export function Select({ label, help, error, required, id, children, ...rest }: SelectProps) {
  const generatedId = useId();
  const chrome: FieldChrome = { label, help, error, required };
  const ids = fieldIds(id ?? generatedId, chrome);

  return (
    <FieldShell chrome={chrome} ids={ids}>
      <select
        {...rest}
        id={ids.controlId}
        className="pv-select"
        required={required}
        aria-invalid={error != null ? true : undefined}
        aria-describedby={ids.describedBy}
      >
        {children}
      </select>
    </FieldShell>
  );
}
