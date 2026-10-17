import { initialsOf } from "./initials";
/** Displays a provider image or name initials, with an optional accessible label. */

import { cx } from "./cx";
import { PersonIcon } from "./icons";

export interface AvatarProps {
  /** `CurrentUser.display_name` — null is expected, not exceptional. */
  name?: string | null;
  /** `CurrentUser.avatar_url` — likewise. */
  src?: string | null;
  /** 32px default, 64px for the profile header. */
  size?: "md" | "lg";
  /** Set when the avatar is the only thing identifying the user, so it needs a
   *  name of its own. Leave unset when a visible name sits beside it — a second
   *  announcement of the same person is noise. */
  label?: string;
  className?: string;
}

/** First letters of the first two words, which is what the design draws ("BP").
 *  Returns null rather than a placeholder character when there is no name. */
export function Avatar({ name, src, size = "md", label, className }: AvatarProps) {
  const initials = initialsOf(name);
  const naming =
    label == null
      ? { "aria-hidden": true as const }
      : { role: "img" as const, "aria-label": label };

  return (
    <span {...naming} className={cx("pv-avatar", size === "lg" && "pv-avatar--lg", className)}>
      {src != null && src !== "" ? (
        <img src={src} alt="" />
      ) : initials != null ? (
        initials
      ) : (
        <PersonIcon size={size === "lg" ? 32 : 18} strokeWidth={1.75} />
      )}
    </span>
  );
}
