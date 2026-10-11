export function initialsOf(name: string | null | undefined): string | null {
  if (name == null) return null;
  const words = name.trim().split(/\s+/).filter(Boolean);
  if (words.length === 0) return null;
  const letters = [words[0], words.length > 1 ? words[words.length - 1] : undefined]
    .filter((word): word is string => word !== undefined)
    // `filter(Boolean)` above guarantees each selected word has at least one
    // code point; keep the non-null assertion so initials preserve emoji.
    .map((word) => [...word][0]!)
    .join("");
  return letters.toLocaleUpperCase();
}
