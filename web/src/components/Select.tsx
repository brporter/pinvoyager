/**
 * Select — the dropdown from `design/components/forms.html`.
 *
 * A native `<select>`, deliberately. A custom listbox would have to
 * reimplement typeahead, keyboard navigation, and the platform's own picker on
 * a phone, and the design has no requirement a native control cannot meet.
 *
 * Note: `design/components.css` styles `.pv-input[aria-invalid="true"]` and
 * `.pv-textarea[aria-invalid="true"]` but has no matching rule for
 * `.pv-select`. The attribute is still set below — assistive technology gets
 * the right answer — but an invalid select will not show the red border until
 * that rule is added to the design system.
 */

import { useId } from "react";
import type { ReactNode, SelectHTMLAttributes } from "react";

import { FieldShell, fieldIds } from "./Field";
import type { FieldChrome } from "./Field";

export interface SelectProps
  extends Omit<SelectHTMLAttributes<HTMLSelectElement>, "className">,
    FieldChrome {
  children: ReactNode;
}

export function Select({
  label,
  help,
  error,
  required,
  id,
  children,
  ...rest
}: SelectProps) {
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
