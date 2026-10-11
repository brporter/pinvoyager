import type { FieldChrome, FieldIds } from "./Field";
export function fieldIds(baseId: string, chrome: FieldChrome): FieldIds {
  const helpId = `${baseId}-help`;
  const errorId = `${baseId}-error`;
  const described = [chrome.error != null && errorId, chrome.help != null && helpId]
    .filter((value): value is string => typeof value === "string")
    .join(" ");

  return {
    controlId: baseId,
    describedBy: described === "" ? undefined : described,
    helpId,
    errorId,
  };
}
