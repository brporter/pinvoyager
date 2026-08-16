/**
 * Avatar — from `design/components/navigation.html`.
 *
 * The initials fallback is not an edge case. Providers do not reliably return a
 * picture, and Apple supplies the user's name only on the very first
 * authorization — so `CurrentUser` has `display_name: string | null` and
 * `avatar_url: string | null`, and an account can legitimately have neither.
 * This component therefore has three renderings, not two: picture, initials,
 * and a person glyph when there is nothing at all to derive initials from.
 *
 * Known design-system gap: `design/components.css` has no rule sizing an
 * `<img>` inside `.pv-avatar`, so a real provider picture is not yet clipped to
 * the circle. The design file mocks the picture case with a CSS gradient class
 * (`pv-img-4`) rather than an image, which is why the rule was never needed.
 */

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
export function initialsOf(name: string | null | undefined): string | null {
  if (name == null) return null;
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;
  const letters = [words[0], words.length > 1 ? words[words.length - 1] : undefined]
    .filter((word): word is string => word !== undefined)
    .map((word) => [...word][0] ?? "")
    .join("");
  return letters.toLocaleUpperCase();
}

export function Avatar({ name, src, size = "md", label, className }: AvatarProps) {
  const initials = initialsOf(name);
  const naming =
    label == null
      ? { "aria-hidden": true as const }
      : { role: "img" as const, "aria-label": label };

  return (
    <span
      {...naming}
      className={cx("pv-avatar", size === "lg" && "pv-avatar--lg", className)}
    >
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
