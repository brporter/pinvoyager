import { Link } from "react-router";

/** Legal links shared by the application and authentication layouts. */
export function SiteFooter() {
  return (
    <footer className="pv-site-footer">
      <nav aria-label="Legal">
        <Link to="/terms">Terms of Service</Link>
        <Link to="/privacy">Privacy Policy</Link>
      </nav>
    </footer>
  );
}
