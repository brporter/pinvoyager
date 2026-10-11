import { useModalDialog } from "./useModalDialog";
import { useId, useState } from "react";
import { useUpdateVoyage } from "../api/hooks";
import type { Voyage } from "../api/types";
import { Button } from "./Button";
import { TextField } from "./TextField";
import { Toast } from "./Toast";
import { CloseIcon } from "./icons";

/** Public sharing follows confirmed publication; native dialog owns modal focus. */
export function VoyageShareDialog({
  voyage,
  canPublish,
  onClose,
}: {
  voyage: Pick<Voyage, "id" | "published">;
  canPublish: boolean;
  onClose: () => void;
}) {
  const modal = useModalDialog(onClose);
  const titleId = useId();
  const update = useUpdateVoyage(voyage.id);
  const [publishedHere, setPublishedHere] = useState(false);
  const [copyState, setCopyState] = useState<"idle" | "copying" | "copied" | "failed">("idle");
  const publicVoyage = voyage.published || publishedHere;
  const url = new URL(`/voyages/${voyage.id}`, window.location.origin).href;

  async function copyUrl() {
    setCopyState("copying");
    try {
      await navigator.clipboard.writeText(url);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  }

  return (
    <dialog {...modal} className="pv-share-dialog" aria-labelledby={titleId} closedby="any">
      <div className="pv-stack-4">
        <header className="pv-inline-2" style={{ justifyContent: "space-between" }}>
          <h2 id={titleId} className="pv-h3" style={{ margin: 0 }}>
            Share voyage
          </h2>
          <form method="dialog">
            <Button
              type="submit"
              variant="ghost"
              size="lg"
              iconOnly
              aria-label="Close share dialog"
            >
              <CloseIcon size={20} />
            </Button>
          </form>
        </header>
        {publicVoyage ? (
          <>
            <p className="pv-sm pv-muted" style={{ margin: 0 }}>
              Anyone with this URL can view this voyage.
            </p>
            <TextField label="Voyage URL" value={url} readOnly />
            <Button
              variant="primary"
              loading={copyState === "copying"}
              disabled={copyState === "copying"}
              onClick={() => void copyUrl()}
            >
              Copy URL
            </Button>
            {copyState === "copied" && (
              <Toast tone="success" title="URL copied">
                The voyage URL has been copied to your clipboard.
              </Toast>
            )}
            {copyState === "failed" && (
              <Toast tone="error" title="The URL could not be copied">
                Copy the URL manually from the field above.
              </Toast>
            )}
          </>
        ) : (
          <>
            <p className="pv-sm pv-muted" style={{ margin: 0 }}>
              Publish this voyage to enable sharing. Published voyages are visible to everyone.
            </p>
            {canPublish ? (
              <Button
                variant="primary"
                loading={update.isPending}
                disabled={update.isPending}
                onClick={() =>
                  update.mutate({ published: true }, { onSuccess: () => setPublishedHere(true) })
                }
              >
                Publish
              </Button>
            ) : (
              <p className="pv-sm pv-muted">Only the owner can publish this voyage.</p>
            )}
            {update.error !== null && (
              <Toast tone="error" title="The voyage could not be published">
                {update.error.message}
              </Toast>
            )}
          </>
        )}
      </div>
    </dialog>
  );
}
