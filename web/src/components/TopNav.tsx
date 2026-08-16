/**
 * TopNav — the desktop bar from `design/components/navigation.html`.
 *
 * Every voyage on PinVoyager is public, so a signed-out visitor is a
 * first-class user rather than someone being held at a gate. The bar offers a
 * way in without demanding one: signed out it shows Discover and a Sign in
 * button, signed in it adds the user's own destinations, a New voyage action
 * and their avatar.
 *
 * Pairs with `TabBar`, which is the same navigation at the mobile breakpoint.
 * The responsive shell renders one or the other, not both — they share the
 * `aria-label="Main"` landmark name, and two landmarks with the same name in
 * one document is a navigation trap.
 *
 * Active state comes from react-router's `NavLink`, which sets
 * `aria-current="page"` — the same attribute `.pv-navlink[aria-current="page"]`
 * styles. One source of truth for "which page am I on", used by both the
 * screen reader and the stylesheet.
 */

import type { ReactNode } from "react";
import { Link, NavLink } from "react-router";

import { Avatar } from "./Avatar";
import { Button } from "./Button";
import { PinIcon, PlusIcon } from "./icons";

export interface NavItem {
  label: string;
  to: string;
  /** Restrict the active match to the exact path. Needed for "/", which would
   *  otherwise match every route. */
  end?: boolean;
}

export interface NavUser {
  displayName: string | null;
  avatarUrl: string | null;
}

export interface TopNavProps {
  /** Defaults to the single public destination. The application shell adds the
   *  signed-in destinations, because it is the thing that knows their paths. */
  links?: NavItem[];
  /** Null or undefined means signed out. */
  user?: NavUser | null;
  brandTo?: string;
  signInTo?: string;
  /** Given, this replaces the sign-in link with a button — for a screen that
   *  opens the provider chooser in place rather than routing to it. */
  onSignIn?: () => void;
  newVoyageTo?: string;
  /** Replaces the default right-hand controls entirely. */
  actions?: ReactNode;
  label?: string;
}

const DEFAULT_LINKS: NavItem[] = [{ label: "Discover", to: "/", end: true }];

export function TopNav({
  links = DEFAULT_LINKS,
  user = null,
  brandTo = "/",
  signInTo = "/signin",
  onSignIn,
  newVoyageTo = "/upload",
  actions,
  label = "Main",
}: TopNavProps) {
  return (
    <nav className="pv-topnav" aria-label={label}>
      <Link className="pv-brand" to={brandTo}>
        <PinIcon className="pv-brand-mark" size={22} />
        PinVoyager
      </Link>

      <div className="pv-navlinks">
        {links.map((link) => (
          <NavLink key={link.to} className="pv-navlink" to={link.to} end={link.end}>
            {link.label}
          </NavLink>
        ))}
      </div>

      <div className="pv-spacer" />

      {actions ??
        (user != null ? (
          <>
            <Link className="pv-btn pv-btn--secondary pv-btn--sm" to={newVoyageTo}>
              <PlusIcon size={14} />
              <span className="pv-btn-label">New voyage</span>
            </Link>
            <Avatar
              name={user.displayName}
              src={user.avatarUrl}
              label={
                user.displayName != null
                  ? `Signed in as ${user.displayName}`
                  : "Signed in"
              }
            />
          </>
        ) : onSignIn != null ? (
          <Button variant="primary" size="sm" onClick={onSignIn}>
            Sign in
          </Button>
        ) : (
          <Link className="pv-btn pv-btn--primary pv-btn--sm" to={signInTo}>
            <span className="pv-btn-label">Sign in</span>
          </Link>
        ))}
    </nav>
  );
}
