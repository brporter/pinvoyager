import { useId, useState } from "react";
import { Link, useNavigate, useParams } from "react-router";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { administerUser, getAdminUser, getAdminUsers } from "../api/client";
import type { AdminAction, AdminUser } from "../api/types";
import { useAuth } from "../auth/useAuth";
import { Button } from "../components/Button";
import { useModalDialog } from "../components/useModalDialog";

type Confirmation = { label: string; description: string; body: AdminAction };

/** The API authorizes independently. Remount on account navigation to clear dialogs and pagination. */
export function Admin() {
  const { user } = useAuth();
  const { id } = useParams();
  if (!user?.is_site_admin)
    return <p className="pv-admin">Site administrator access is required.</p>;
  return <AdminContent key={id ?? "list"} id={id} actorId={user.id} />;
}

function storageSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let size = bytes / 1024;
  let unit = 0;
  while (size >= 1024 && unit < units.length - 1) {
    size /= 1024;
    unit++;
  }
  return `${size.toFixed(1)} ${units[unit]}`;
}

function AccountStats({ user }: { user: AdminUser }) {
  return (
    <p className="pv-sm pv-muted">
      {user.photo_count} photos · {user.voyage_count} voyages · {storageSize(user.storage_bytes)}{" "}
      stored · {user.suspended ? "Suspended" : "Active"}
      {user.is_site_admin ? " · Site administrator" : ""}
    </p>
  );
}

function AdminContent({ id, actorId }: { id?: string; actorId: string }) {
  const [cursors, setCursors] = useState<(string | undefined)[]>([undefined]);
  const before = cursors[cursors.length - 1];
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const [notice, setNotice] = useState("");
  const navigate = useNavigate();
  const client = useQueryClient();
  const list = useQuery({
    queryKey: ["admin", "users", before],
    queryFn: () => getAdminUsers(before),
    enabled: id === undefined,
  });
  const detail = useQuery({
    queryKey: ["admin", "user", id, before],
    queryFn: () => getAdminUser(id!, before),
    enabled: id !== undefined,
  });
  const query = id === undefined ? list : detail;
  const next = query.data?.next_before;
  const account = detail.data?.user;
  const mutation = useMutation({
    mutationFn: (body: AdminAction) => administerUser(id!, body),
    onSuccess: async (_, body) => {
      setConfirmation(null);
      setNotice("Action completed.");
      // Account moderation also changes public feeds, voyage pages, and profiles.
      if (body.action === "delete") {
        client.removeQueries({ queryKey: ["admin", "user", id] });
        void navigate("/admin", { replace: true });
      }
      await client.invalidateQueries();
    },
  });
  function ask(value: Confirmation) {
    mutation.reset();
    setNotice("");
    setConfirmation(value);
  }

  return (
    <section className="pv-admin pv-stack-4">
      <header>
        {id && <Link to="/admin">All users</Link>}
        <Link to="/admin/reports">Content reports</Link>
        <h1 className="pv-h2">{id ? "User administration" : "Site administration"}</h1>
        <p className="pv-muted">
          Storage includes original uploads and generated images. Shared content counts once per
          account. Totals may be up to a minute old.
        </p>
      </header>
      {notice && <p role="status">{notice}</p>}
      {query.isPending ? (
        <p role="status">Loading accounts…</p>
      ) : query.isError ? (
        <div role="alert">
          <p>{query.error.message}</p>
          <Button onClick={() => void query.refetch()}>Try again</Button>
        </div>
      ) : (
        <>
          {id === undefined && list.data && (
            <>
              <h2 className="pv-h3">Users</h2>
              {list.data.users.length === 0 ? (
                <p>No users found.</p>
              ) : (
                <div className="pv-admin-table-scroll">
                  <table className="pv-admin-table">
                    <thead>
                      <tr>
                        <th scope="col">Account</th>
                        <th scope="col">Status</th>
                        <th scope="col">Role</th>
                        <th scope="col">Photos</th>
                        <th scope="col">Voyages</th>
                        <th scope="col">Storage</th>
                      </tr>
                    </thead>
                    <tbody>
                      {list.data.users.map((user) => (
                        <tr key={user.id}>
                          <th scope="row">
                            <Link to={`/admin/users/${user.id}`}>
                              {user.display_name ?? user.id}
                            </Link>
                          </th>
                          <td>{user.suspended ? "Suspended" : "Active"}</td>
                          <td>{user.is_site_admin ? "Site administrator" : "User"}</td>
                          <td>{user.photo_count}</td>
                          <td>{user.voyage_count}</td>
                          <td>{storageSize(user.storage_bytes)}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
              )}
            </>
          )}
          {account && (
            <>
              <div className="pv-card pv-admin-card">
                <h2 className="pv-h3">{account.display_name ?? account.id}</h2>
                <p className="pv-sm pv-muted">Account ID: {account.id}</p>
                <AccountStats user={account} />
                <div className="pv-admin-actions">
                  <Button
                    disabled={actorId === account.id}
                    onClick={() =>
                      ask({
                        label: account.suspended ? "Enable account" : "Suspend account",
                        description: account.suspended
                          ? "Allow this user to sign in again. They must start a new session."
                          : "Revoke this user's sessions and prevent sign-in. Published voyages remain public until unpublished.",
                        body: { action: account.suspended ? "enable" : "suspend" },
                      })
                    }
                  >
                    {account.suspended ? "Enable account" : "Suspend account"}
                  </Button>
                  {!account.is_site_admin && (
                    <Button
                      onClick={() =>
                        ask({
                          label: "Promote to administrator",
                          description:
                            "Give this user access to all account administration actions, including deletion and promotion of other accounts.",
                          body: { action: "promote" },
                        })
                      }
                    >
                      Promote to administrator
                    </Button>
                  )}
                  <Button
                    onClick={() =>
                      ask({
                        label: "Unpublish all voyages",
                        description:
                          "Make every voyage owned by this user private. An enabled owner can publish them again.",
                        body: { action: "unpublish_all" },
                      })
                    }
                  >
                    Unpublish all voyages
                  </Button>
                  <Button
                    variant="danger"
                    disabled={actorId === account.id}
                    onClick={() =>
                      ask({
                        label: "Delete account and all data",
                        description:
                          "Permanently delete this account, its identities, sessions, voyages, and photos. Unshared files are removed; failed file removals are retried. Display names remain reserved. This cannot be undone.",
                        body: { action: "delete" },
                      })
                    }
                  >
                    Delete account and all data
                  </Button>
                </div>
                {actorId === account.id && (
                  <p className="pv-sm pv-muted">
                    You cannot suspend or delete your own administrator account.
                  </p>
                )}
              </div>
              <h2 className="pv-h3">Voyages</h2>
              {detail.data!.voyages.length === 0 && <p>No voyages found.</p>}
              {detail.data!.voyages.map((voyage) => (
                <article className="pv-card pv-admin-card" key={voyage.id}>
                  <h3 className="pv-h3">{voyage.title}</h3>
                  <p style={{ whiteSpace: "pre-wrap" }}>
                    {voyage.description ?? "No description."}
                  </p>
                  <p className="pv-sm pv-muted">{voyage.published ? "Published" : "Draft"}</p>
                  {voyage.published && (
                    <Button
                      aria-label={`Unpublish ${voyage.title}`}
                      onClick={() =>
                        ask({
                          label: `Unpublish ${voyage.title}`,
                          description:
                            "Make this voyage private. An enabled owner can publish it again.",
                          body: { action: "unpublish_voyage", voyage_id: voyage.id },
                        })
                      }
                    >
                      Unpublish voyage
                    </Button>
                  )}
                </article>
              ))}
            </>
          )}
          <nav aria-label="Pagination" className="pv-inline-2">
            <Button
              disabled={cursors.length === 1 || query.isFetching}
              onClick={() => setCursors((values) => values.slice(0, -1))}
            >
              Previous page
            </Button>
            <span>Page {cursors.length}</span>
            <Button
              disabled={!next || query.isFetching}
              onClick={() => setCursors((values) => [...values, next!])}
            >
              Next page
            </Button>
          </nav>
        </>
      )}
      {confirmation && account && (
        <ConfirmAction
          confirmation={confirmation}
          account={account}
          pending={mutation.isPending}
          error={mutation.error}
          onCancel={() => setConfirmation(null)}
          onConfirm={() => mutation.mutate(confirmation.body)}
        />
      )}
    </section>
  );
}

function ConfirmAction({
  confirmation,
  account,
  pending,
  error,
  onCancel,
  onConfirm,
}: {
  confirmation: Confirmation;
  account: AdminUser;
  pending: boolean;
  error: Error | null;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const modal = useModalDialog(onCancel);
  const title = useId();
  const description = useId();
  return (
    <dialog
      {...modal}
      onClick={pending ? undefined : modal.onClick}
      className="pv-share-dialog"
      aria-labelledby={title}
      aria-describedby={description}
      onCancel={(event) => {
        if (pending) event.preventDefault();
      }}
    >
      <div className="pv-stack-4">
        <h2 id={title} className="pv-h3">
          {confirmation.label}?
        </h2>
        <p>Account: {account.display_name ?? account.id}</p>
        <p id={description}>{confirmation.description}</p>
        {error && <p role="alert">{error.message}</p>}
        <div className="pv-inline-2">
          <Button autoFocus disabled={pending} onClick={onCancel}>
            Cancel
          </Button>
          <Button
            variant={confirmation.body.action === "delete" ? "danger" : "primary"}
            loading={pending}
            disabled={pending}
            onClick={onConfirm}
          >
            Confirm
          </Button>
        </div>
      </div>
    </dialog>
  );
}
