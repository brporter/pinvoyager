/** Renders a labeled switch with pointer and keyboard activation. */

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
