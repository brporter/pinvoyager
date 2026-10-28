import { useState } from "react";
import { Link } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getReports, resolveReport } from "../api/client";
import type { ContentReport, ReportResolution } from "../api/types";
import { useAuth } from "../auth/useAuth";
import { Button, TextArea } from "../components";

export function Moderation() {
  const { user } = useAuth();
  return user?.is_site_admin ? <ReportQueue /> : <p>Site administrator access is required.</p>;
}
function ReportQueue() {
  const client = useQueryClient();
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const before = cursors[cursors.length - 1];
  const [selected, setSelected] = useState<ContentReport | null>(null);
  const [note, setNote] = useState("");
  const [notice, setNotice] = useState("");
  const query = useQuery({
    queryKey: ["admin", "reports", before],
    queryFn: () => getReports(before),
  });
  const mutation = useMutation({
    mutationFn: (action: ReportResolution["action"]) =>
      resolveReport(selected!.id, { action, note: note.trim() }),
    onSuccess: async () => {
      setSelected(null);
      setNote("");
      setNotice("Review saved.");
      await client.invalidateQueries();
    },
  });
  function review(report: ContentReport) {
    setSelected(report);
    setNote("");
    setNotice("");
    mutation.reset();
  }
  return (
    <section className="pv-admin pv-stack-4">
      <Link to="/admin">Account administration</Link>
      <h1>Content reports</h1>
      <p>
        Report details and review notes are private. Holding content removes public access and
        prevents publication. Release allows the owner to publish again.
      </p>
      {notice && <p role="status">{notice}</p>}
      {query.isPending ? (
        <p role="status">Loading reports…</p>
      ) : query.isError ? (
        <p role="alert">
          {query.error.message} <Button onClick={() => void query.refetch()}>Retry reports</Button>
        </p>
      ) : (
        <>
          {query.data.reports.length === 0 && <p>No reports.</p>}
          <ul>
            {query.data.reports.map((report) => (
              <li key={report.id}>
                <Button variant="ghost" onClick={() => review(report)}>
                  Review {report.reason}
                </Button>{" "}
                · {report.status}
              </li>
            ))}
          </ul>
          <div className="pv-inline-2">
            <Button
              disabled={cursors.length === 1}
              onClick={() => {
                setCursors((values) => values.slice(0, -1));
                setSelected(null);
              }}
            >
              Previous page
            </Button>
            <Button
              disabled={!query.data.next_before}
              onClick={() => {
                setCursors((values) => [...values, query.data.next_before!]);
                setSelected(null);
              }}
            >
              Next page
            </Button>
          </div>
        </>
      )}
      {selected && (
        <section className="pv-card pv-stack-4" aria-label="Report review">
          <h2>{selected.reason}</h2>
          <p>{selected.details}</p>
          <p>Target: {selected.target}</p>
          <p>Status: {selected.status}</p>
          {selected.moderator_note && <p>Previous review: {selected.moderator_note}</p>}
          {selected.target_owner_id && (
            <Link to={`/admin/users/${selected.target_owner_id}`}>Review traveler account</Link>
          )}
          <TextArea
            label="Review note"
            value={note}
            maxLength={2000}
            onChange={(event) => setNote(event.target.value)}
          />
          <div className="pv-inline-2">
            {(
              [
                ["hold", "Hold content"],
                ["release", "Release content"],
                ["dismiss", "Dismiss report"],
              ] as const
            ).map(([action, label]) => (
              <Button
                key={action}
                disabled={!note.trim() || mutation.isPending}
                onClick={() => mutation.mutate(action)}
              >
                {label}
              </Button>
            ))}
            <Button variant="ghost" onClick={() => setSelected(null)}>
              Close review
            </Button>
          </div>
          {mutation.error && <p role="alert">{mutation.error.message}</p>}
        </section>
      )}
    </section>
  );
}
