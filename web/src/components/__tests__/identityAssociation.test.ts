/// <reference types="node" />
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("publishes the Microsoft application association at the well-known path", () => {
  const association = readFileSync(
    "public/.well-known/microsoft-identity-association.json",
    "utf8",
  );

  expect(JSON.parse(association)).toEqual({
    associatedApplications: [{ applicationId: "857dc096-adfb-420c-911d-7734b26dd270" }],
  });
});
