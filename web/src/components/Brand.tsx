/** Links to the site home with the Horizon Pin logo and site name. */
import { Link } from "react-router";
import horizonPin from "../../../design/logos/horizon-pin.svg";

export function Brand({ to = "/" }: { to?: string }) {
  return (
    <Link className="pv-brand" to={to}>
      <img className="pv-brand-mark" src={horizonPin} width={22} height={22} alt="" />
      PinVoyager
    </Link>
  );
}
