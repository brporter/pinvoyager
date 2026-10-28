import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { cancelPublicationIntent, getPublicationIntents } from "../api/client";
import { Button } from "./Button";

/** The server owns consent and finalization; device state cannot cancel an armed request. */
export function PublicationPanel({ voyageId }: { voyageId: string }) {
  const client = useQueryClient();
  const [notice, setNotice] = useState("");
  const query = useQuery({
    queryKey: ["publication-intents", voyageId],
    queryFn: () => getPublicationIntents(voyageId),
    refetchInterval: 15000,
  });
  const cancel = useMutation({
    mutationFn: (id: string) => cancelPublicationIntent(voyageId, id),
    onSuccess: async (result) => {
      setNotice(
        result.published
          ? "This voyage already published. Use Unpublish to make it private."
          : "Publication is no longer scheduled.",
      );
      await client.invalidateQueries();
    },
  });
  return (
    <section className="pv-stack-3" aria-label="Scheduled publication">
      {notice && <p role="status">{notice}</p>}
      {query.error && (
        <p role="alert">
          Scheduled publication could not be checked.{" "}
          <Button variant="ghost" onClick={() => void query.refetch()}>
            Check publication
          </Button>
        </p>
      )}
      {query.data?.intents.map((intent) => (
        <div className="pv-card" key={intent.id}>
          <h2>Publication scheduled from the app</h2>
          <p>
            {intent.waiting_count} uploads remaining · {intent.processing_count} photos processing
          </p>
          <p>
            The reviewed version can publish while the app is closed. Saving changes invalidates
            this review.
          </p>
          <Button disabled={cancel.isPending} onClick={() => cancel.mutate(intent.id)}>
            Cancel scheduled publication
          </Button>
        </div>
      ))}
      {cancel.error && <p role="alert">{cancel.error.message}</p>}
    </section>
  );
}
