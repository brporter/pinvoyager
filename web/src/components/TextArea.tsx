/** Renders a textarea with shared field labeling and validation feedback. */

import { useId } from "react";
import type { TextareaHTMLAttributes } from "react";

import { FieldShell } from "./Field";
import { fieldIds } from "./fieldIds";
import type { FieldChrome } from "./Field";

export interface TextAreaProps
  extends Omit<TextareaHTMLAttributes<HTMLTextAreaElement>, "className">, FieldChrome {}

export function TextArea({ label, help, error, required, id, ...rest }: TextAreaProps) {
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
