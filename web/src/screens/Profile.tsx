/** Resolve /u/:identifier to the owner workspace or published voyages. */
import { useParams } from "react-router";
import { useAuth } from "../auth/useAuth";
import { VoyageGridSkeleton } from "../components/VoyageCards";
import { useIsDesktop } from "../shell/useIsDesktop";
import { PublicProfile } from "./PublicProfile";
import { OwnerVoyages } from "./OwnerVoyages";

export function Profile() {
  const { identifier } = useParams<{ identifier: string }>();
  const auth = useAuth();
  const desktop = useIsDesktop();
  if (auth.status === "loading") return <VoyageGridSkeleton desktop={desktop} />;
  if (
    auth.user &&
    (auth.user.id === identifier?.toLowerCase() ||
      auth.user.display_name === identifier?.toLowerCase())
  ) {
    return <OwnerVoyages owner={auth.user} desktop={desktop} />;
  }
  return <PublicProfile />;
}
