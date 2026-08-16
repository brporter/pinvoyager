/**
 * Class name joiner.
 *
 * The only styling utility in this directory. Every component in
 * `src/components` styles itself by applying class names that already exist in
 * `design/components.css` — none of them define what those class names mean.
 * See `src/styles/index.css` for why.
 */
export type ClassValue = string | false | null | undefined;

export function cx(...values: ClassValue[]): string {
  return values.filter(Boolean).join(" ");
}
