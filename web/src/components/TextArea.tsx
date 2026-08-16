/**
 * TextArea — the multi-line input from `design/components/forms.html`.
 *
 * `.pv-textarea` allows vertical resizing only. Horizontal resize would let a
 * user drag the control out of its column and break the form's layout, and it
 * is never what they wanted.
 */

import { useId } from "react";
import type { TextareaHTMLAttributes } from "react";

import { FieldShell, fieldIds } from "./Field";
import type { FieldChrome } from "./Field";

export interface TextAreaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "className">,
    FieldChrome {}

export function TextArea({
  label,
  help,
  error,
  required,
  id,
  ...rest
}: TextAreaProps) {
  const generatedId = useId();
  const chrome: FieldChrome = { label, help, error, required };
  const ids = fieldIds(id ?? generatedId, chrome);

  return (
    <FieldShell chrome={chrome} ids={ids}>
      <textarea
        {...rest}
        id={ids.controlId}
        className="pv-textarea"
        required={required}
        aria-invalid={error != null ? true : undefined}
        aria-describedby={ids.describedBy}
      />
    </FieldShell>
  );
}
