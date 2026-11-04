/** Links to the site home with the iOS app artwork and site name. */
import { Link } from "react-router";

export function BrandIcon() {
  return (
    <picture className="pv-brand-mark">
      <source media="(prefers-color-scheme: dark)" srcSet="/brand/pinvoyager-dark.png" />
      <img src="/brand/pinvoyager-light.png" width={32} height={32} alt="" />
    </picture>
  );
}

export function Brand({ to = "/" }: { to?: string }) {
  return (
    <Link className="pv-brand" to={to}>
      <BrandIcon />
      PinVoyager
    </Link>
  );
}
