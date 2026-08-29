/**
 * Checkbox — from `design/components/forms.html`.
 *
 * A real `<input type="checkbox">` visually hidden with `.pv-sr`, beside a
 * `.pv-check-box` that draws the state. The input stays in the accessibility
 * tree and stays focusable, so keyboard operation, form participation and
 * screen-reader announcement are all the platform's rather than ours; only the
 * appearance is borrowed from the design system.
 *
 * State is signalled by a shape change (the tick) as well as a fill color, so
 * it survives without color perception.
 */

import { useId, useState } from "react";
import type { ChangeEvent, InputHTMLAttributes, ReactNode } from "react";

import { CheckIcon } from "./icons";

export interface CheckboxProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "className" | "type" | "children"> {
  /** Visible label text. */
  children: ReactNode;
}

export function Checkbox({
  children,
  checked,
  defaultChecked,
  onChange,
  id,
  ...rest
}: CheckboxProps) {
  const generatedId = useId();
  const controlId = id ?? generatedId;

  // The tick is rendered conditionally, so an uncontrolled checkbox needs its
  // state mirrored here. Without this the component would silently only work
  // when someone remembered to pass `checked`.
  const [internal, setInternal] = useState(defaultChecked ?? false);
  const isControlled = checked !== undefined;
  const isChecked = isControlled ? checked : internal;

  return (
    <label className="pv-check" htmlFor={controlId}>
      <input
        {...rest}
        id={controlId}
        type="checkbox"
        className="pv-sr"
        checked={isControlled ? checked : undefined}
        defaultChecked={isControlled ? undefined : defaultChecked}
        onChange={(event: ChangeEvent<HTMLInputElement>) => {
          if (!isControlled) setInternal(event.target.checked);
          onChange?.(event);
        }}
      />
      <span className="pv-check-box">
        {/* The design file hardcodes `stroke="#fff"`. The token is used instead
            because `--pv-text-on-accent` flips to near-black in dark mode,
            where the accent fill is light — a white tick would disappear. */}
        {isChecked && (
          <CheckIcon size={12} stroke="var(--pv-text-on-accent)" strokeWidth={3.5} />
        )}
      </span>
      <span className="pv-check-text">{children}</span>
    </label>
  );
}
