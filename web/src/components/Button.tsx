/**
 * Button — implements `design/components/buttons.html`.
 *
 * Four variants, three sizes, plus the icon-only, loading and disabled states
 * drawn in the design file. Every appearance below comes from `.pv-btn` and its
 * modifiers in `design/components.css`; this file chooses which of those class
 * names apply and nothing else.
 *
 * Two behaviours are worth knowing about:
 *
 *  - **Icon-only requires an accessible name.** The type below will not compile
 *    without an `aria-label` when `iconOnly` is set. An icon with no accessible
 *    name is unusable with a screen reader, and it is the most common
 *    accessibility defect in interfaces of this kind.
 *  - **Loading suppresses the click.** The design system sets
 *    `pointer-events: none` on a loading button, which stops the mouse but not
 *    the keyboard — Enter on a focused button still fires. The handler is
 *    guarded here so "loading" means the same thing to both.
 */

import type { ButtonHTMLAttributes, MouseEvent, ReactNode } from "react";

import { cx } from "./cx";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

interface ButtonBase
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  variant?: ButtonVariant;
  /** 32px, 40px (default), 48px. All three keep a 44px touch target. */
  size?: ButtonSize;
  /** Shows the spinner beside the label and blocks activation. The label stays,
   *  so the button keeps its width and the layout does not jump. */
  loading?: boolean;
  children?: ReactNode;
  /** Escape hatch for layout classes from the design system (`pv-spacer` and
   *  friends). Not a place to hang new styles. */
  className?: string;
}

interface ButtonWithLabel extends ButtonBase {
  iconOnly?: false;
  /** Rendered before the label, replaced by the spinner while loading. */
  icon?: ReactNode;
}

interface ButtonIconOnly extends ButtonBase {
  iconOnly: true;
  /** Required: the icon is the whole button, so it is the only place an
   *  accessible name can come from. */
  "aria-label": string;
  icon?: never;
}

export type ButtonProps = ButtonWithLabel | ButtonIconOnly;

export function Button(props: ButtonProps) {
  const {
    variant = "secondary",
    size = "md",
    loading = false,
    iconOnly = false,
    icon,
    className,
    children,
    disabled,
    type = "button",
    onClick,
    ...rest
  } = props as ButtonBase & { iconOnly?: boolean; icon?: ReactNode };

  const inert = loading || disabled === true;

  return (
    <button
      {...rest}
      type={type}
      disabled={disabled}
      aria-busy={loading || undefined}
      data-loading={loading ? "true" : undefined}
      onClick={(event: MouseEvent<HTMLButtonElement>) => {
        if (inert) {
          event.preventDefault();
          return;
        }
        onClick?.(event);
      }}
      className={cx(
        "pv-btn",
        `pv-btn--${variant}`,
        size !== "md" && `pv-btn--${size}`,
        iconOnly && "pv-btn--icon",
        className,
      )}
    >
      {loading ? <Spinner /> : icon}
      {children == null ? null : iconOnly ? (
        children
      ) : (
        <span className="pv-btn-label">{children}</span>
      )}
    </button>
  );
}

/**
 * The `.pv-spinner` from the design system. Purely decorative — the state it
 * represents is announced by `aria-busy` on the control that owns it, so a
 * screen reader hears "busy" rather than a nameless graphic.
 */
export function Spinner({ className }: { className?: string }) {
  return <span className={cx("pv-spinner", className)} aria-hidden="true" />;
}
