/**
 * Switch — from `design/components/forms.html`.
 *
 * ## Why this is a div and not a button
 *
 * `.pv-switch` in `design/components.css` is written for a `<div>`: it sets a
 * background, a size and a `::after` knob, but none of the resets a `<button>`
 * needs (`border: 0`, `padding: 0`, `appearance: none`). Rendering a `<button>`
 * with that class would pick up the user agent's border and padding and come
 * out the wrong size, so the design file's `role="switch"` + `tabindex="0"`
 * markup is followed exactly and the keyboard behaviour a button would have
 * given for free is implemented here: Space and Enter both toggle.
 *
 * State reaches assistive technology through `aria-checked`, and reaches
 * everyone else through the knob's position as well as the track's colour —
 * two signals, neither of which is colour alone. The adjacent `stateText`
 * ("Draft" / "Published — visible to everyone") is the third and most reliable.
 */

import type { KeyboardEvent, ReactNode } from "react";

import { cx } from "./cx";

export interface SwitchProps {
  checked: boolean;
  onCheckedChange: (checked: boolean) => void;
  /** Accessible name. Required — the track is a graphic with no text of its
   *  own, so this is the only name a screen reader can announce. */
  label: string;
  /** Visible text beside the switch, which should describe the current state
   *  rather than the control. */
  stateText?: ReactNode;
  disabled?: boolean;
  id?: string;
  className?: string;
}

export function Switch({
  checked,
  onCheckedChange,
  label,
  stateText,
  disabled = false,
  id,
  className,
}: SwitchProps) {
  function toggle() {
    if (disabled) return;
    onCheckedChange(!checked);
  }

  const control = (
    <div
      id={id}
      role="switch"
      aria-checked={checked}
      aria-label={label}
      aria-disabled={disabled || undefined}
      tabIndex={disabled ? -1 : 0}
      data-on={checked ? "true" : "false"}
      className={cx("pv-switch", className)}
      onClick={toggle}
      onKeyDown={(event: KeyboardEvent<HTMLDivElement>) => {
        // Space is the switch role's activation key; Enter is accepted too
        // because a real <button> would have accepted it and people expect it.
        if (event.key === " " || event.key === "Enter") {
          event.preventDefault();
          toggle();
        }
      }}
    />
  );

  if (stateText == null) return control;

  return (
    <div className="pv-inline-3">
      {control}
      <span className="pv-sm">{stateText}</span>
    </div>
  );
}
