/**
 * ProviderButton — implements `design/components/provider-buttons.html`.
 *
 * The only way into PinVoyager. Four providers, one shared geometry, four
 * mandated skins.
 *
 * ## Do not restyle or redraw these
 *
 * Google and Apple both publish binding branding requirements: exact wordmark,
 * minimum size, permitted colour variants, required clear space.
 * Non-compliance is grounds for rejection during provider app review, which
 * blocks launch. The four SVGs below are copied verbatim from the design file —
 * treat any change as a correctness question rather than a taste question, and
 * check the current published guidelines first.
 *
 * The skins live in `.pv-provider--*` in `design/components.css`, including the
 * per-provider hex values the guidelines mandate. Those are the one place in
 * the design system where literal colours are permitted, precisely because they
 * are not ours to tokenise.
 */

import type { ButtonHTMLAttributes, ReactElement } from "react";

import { cx } from "./cx";

/** Matches `LinkedProvider["provider"]` in `src/api/types.ts`. */
export type Provider = "google" | "apple" | "microsoft" | "facebook";

const PROVIDER_NAME: Record<Provider, string> = {
  google: "Google",
  apple: "Apple",
  microsoft: "Microsoft",
  facebook: "Facebook",
};

export interface ProviderButtonProps
  extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "className"> {
  provider: Provider;
  /** The redirect has been initiated. Swaps the logo for a spinner and blocks
   *  a second activation, because two OAuth redirects race each other. */
  loading?: boolean;
  /** Overrides "Continue with X". The shared label grammar is deliberate — it
   *  makes the sign-in screen read as one set of equivalent choices rather than
   *  four vendors competing — so override it only with good reason. */
  label?: string;
  className?: string;
}

export function ProviderButton({
  provider,
  loading = false,
  label,
  disabled,
  type = "button",
  onClick,
  className,
  ...rest
}: ProviderButtonProps) {
  const name = PROVIDER_NAME[provider];
  const Logo = PROVIDER_LOGO[provider];
  const text = label ?? (loading ? `Redirecting to ${name}…` : `Continue with ${name}`);
  const inert = loading || disabled === true;

  return (
    <button
      {...rest}
      type={type}
      disabled={disabled}
      aria-busy={loading || undefined}
      data-loading={loading ? "true" : undefined}
      onClick={(event) => {
        if (inert) {
          event.preventDefault();
          return;
        }
        onClick?.(event);
      }}
      className={cx("pv-provider", `pv-provider--${provider}`, className)}
    >
      {loading ? (
        // `pv-provider-icon` sizes the spinner to the 20px logo slot, so the
        // label does not shift sideways when the logo is replaced.
        <span className="pv-spinner pv-provider-icon" aria-hidden="true" />
      ) : (
        <Logo />
      )}
      <span className="pv-provider-label">{text}</span>
    </button>
  );
}

/* ==========================================================================
   Provider logos — copied verbatim from design/components/provider-buttons.html
   ========================================================================== */

function GoogleLogo() {
  return (
    <svg className="pv-provider-icon" viewBox="0 0 48 48" aria-hidden="true" focusable="false">
      <path
        fill="#4285F4"
        d="M45.12 24.5c0-1.56-.14-3.06-.4-4.5H24v8.51h11.84c-.51 2.75-2.06 5.08-4.39 6.64v5.52h7.11c4.16-3.83 6.56-9.47 6.56-16.17z"
      />
      <path
        fill="#34A853"
        d="M24 46c5.94 0 10.92-1.97 14.56-5.33l-7.11-5.52c-1.97 1.32-4.49 2.1-7.45 2.1-5.73 0-10.58-3.87-12.31-9.07H4.34v5.7C7.96 41.07 15.4 46 24 46z"
      />
      <path
        fill="#FBBC05"
        d="M11.69 28.18C11.25 26.86 11 25.45 11 24s.25-2.86.69-4.18v-5.7H4.34C2.85 17.09 2 20.45 2 24s.85 6.91 2.34 9.88l7.35-5.7z"
      />
      <path
        fill="#EA4335"
        d="M24 10.75c3.23 0 6.13 1.11 8.41 3.29l6.31-6.31C34.91 4.18 29.93 2 24 2 15.4 2 7.96 6.93 4.34 14.12l7.35 5.7c1.73-5.2 6.58-9.07 12.31-9.07z"
      />
    </svg>
  );
}

function AppleLogo() {
  return (
    <svg
      className="pv-provider-icon"
      viewBox="0 0 384 512"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M318.7 268.7c-.2-36.7 16.4-64.4 50-84.8-18.8-26.9-47.2-41.7-84.7-44.6-35.5-2.8-74.3 20.7-88.5 20.7-15 0-49.4-19.7-76.4-19.7C63.3 141.2 4 184.8 4 273.5q0 39.3 14.4 81.2c12.8 36.7 59 126.7 107.2 125.2 25.2-.6 43-17.9 75.8-17.9 31.8 0 48.3 17.9 76.4 17.9 48.6-.7 90.4-82.5 102.6-119.3-65.2-30.7-61.7-90-61.7-91.9zm-56.6-164.2c27.3-32.4 24.8-61.9 24-72.5-24.1 1.4-52 16.4-67.9 34.9-17.5 19.8-27.8 44.3-25.6 71.9 26.1 2 49.9-11.4 69.5-34.3z" />
    </svg>
  );
}

function MicrosoftLogo() {
  return (
    <svg className="pv-provider-icon" viewBox="0 0 23 23" aria-hidden="true" focusable="false">
      <rect x="1" y="1" width="10" height="10" fill="#f25022" />
      <rect x="12" y="1" width="10" height="10" fill="#7fba00" />
      <rect x="1" y="12" width="10" height="10" fill="#00a4ef" />
      <rect x="12" y="12" width="10" height="10" fill="#ffb900" />
    </svg>
  );
}

function FacebookLogo() {
  return (
    <svg
      className="pv-provider-icon"
      viewBox="0 0 24 24"
      fill="currentColor"
      aria-hidden="true"
      focusable="false"
    >
      <path d="M9.101 23.691v-7.98H6.627v-3.667h2.474v-1.58c0-4.085 1.848-5.978 5.858-5.978.401 0 .955.042 1.468.103a8.68 8.68 0 0 1 1.141.195v3.325a8.623 8.623 0 0 0-.653-.036 26.805 26.805 0 0 0-.733-.009c-.707 0-1.259.096-1.675.309a1.686 1.686 0 0 0-.679.622c-.258.42-.374.995-.374 1.752v1.297h3.919l-.386 2.103-.287 1.564h-3.246v8.245C19.396 23.238 24 18.179 24 12.044c0-6.627-5.373-12-12-12s-12 5.373-12 12c0 5.628 3.874 10.35 9.101 11.647Z" />
    </svg>
  );
}

const PROVIDER_LOGO: Record<Provider, () => ReactElement> = {
  google: GoogleLogo,
  apple: AppleLogo,
  microsoft: MicrosoftLogo,
  facebook: FacebookLogo,
};

/** The four providers in the order the sign-in screen shows them — by expected
 *  usage, as the design file specifies. */
export const PROVIDERS: readonly Provider[] = [
  "google",
  "apple",
  "microsoft",
  "facebook",
];
