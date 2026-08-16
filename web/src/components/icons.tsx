/**
 * The icon set drawn in `design/components/`.
 *
 * Every path here is copied from a design file rather than redrawn, so the
 * component and the specimen it implements are the same shape. Icons are
 * always `aria-hidden` and `focusable={false}`: they are decoration beside an
 * accessible name, never the name itself. That is why `Button` requires an
 * `aria-label` when it renders nothing but an icon.
 *
 * Colour comes from `currentColor`, so an icon inherits whatever the
 * design-system class on its container decided. No icon sets its own colour.
 */

import type { ReactNode, SVGProps } from "react";

export interface IconProps extends Omit<SVGProps<SVGSVGElement>, "children"> {
  /** Width and height in pixels. Design files use 12–40 depending on context. */
  size?: number;
}

function icon(
  viewBox: string,
  body: ReactNode,
  defaults: SVGProps<SVGSVGElement> = {},
) {
  return function Icon({ size = 16, ...rest }: IconProps) {
    return (
      <svg
        viewBox={viewBox}
        width={size}
        height={size}
        fill="none"
        stroke="currentColor"
        strokeWidth={2}
        strokeLinecap="round"
        strokeLinejoin="round"
        aria-hidden="true"
        focusable="false"
        {...defaults}
        {...rest}
      >
        {body}
      </svg>
    );
  };
}

/* -- buttons.html ------------------------------------------------------- */

export const EditIcon = icon(
  "0 0 24 24",
  <>
    <path d="M12 20h9" />
    <path d="M16.5 3.5a2.12 2.12 0 0 1 3 3L7 19l-4 1 1-4Z" />
  </>,
);

export const ShareIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="18" cy="5" r="3" />
    <circle cx="6" cy="12" r="3" />
    <circle cx="18" cy="19" r="3" />
    <path d="m8.6 13.5 6.8 4M15.4 6.5l-6.8 4" />
  </>,
);

export const MoreIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="12" cy="5" r="2" />
    <circle cx="12" cy="12" r="2" />
    <circle cx="12" cy="19" r="2" />
  </>,
  { fill: "currentColor", stroke: "none" },
);

export const UploadIcon = icon(
  "0 0 24 24",
  <>
    <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
    <path d="m7 10 5-5 5 5" />
    <path d="M12 5v12" />
  </>,
);

export const PlusIcon = icon("0 0 24 24", <path d="M12 5v14M5 12h14" />);

export const CloseIcon = icon("0 0 24 24", <path d="M18 6 6 18M6 6l12 12" />);

/* -- forms.html / feedback.html ----------------------------------------- */

export const AlertCircleIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="12" cy="12" r="10" />
    <path d="M12 8v5M12 16.5v.01" />
  </>,
);

/** The dropzone-reject and upload-error variant: a tighter circle with a
 *  9 radius, as drawn in upload.html. */
export const ErrorCircleIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 8v5M12 16.5v.01" />
  </>,
);

export const CircleXIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="12" cy="12" r="9" />
    <path d="m15 9-6 6M9 9l6 6" />
  </>,
);

export const CheckIcon = icon("0 0 24 24", <path d="M20 6 9 17l-5-5" />);

export const WarningTriangleIcon = icon(
  "0 0 24 24",
  <>
    <path d="M12 3 2 20h20L12 3Z" />
    <path d="M12 10v4M12 17v.01" />
  </>,
);

/* -- navigation.html ---------------------------------------------------- */

export const PinIcon = icon(
  "0 0 24 24",
  <>
    <path d="M12 21s-7-6.2-7-11a7 7 0 1 1 14 0c0 4.8-7 11-7 11Z" />
    <circle cx="12" cy="10" r="2.5" />
  </>,
);

export const SearchIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="11" cy="11" r="7" />
    <path d="m20 20-3.5-3.5" />
  </>,
);

export const PersonIcon = icon(
  "0 0 24 24",
  <>
    <circle cx="12" cy="8" r="4" />
    <path d="M4 21a8 8 0 0 1 16 0" />
  </>,
);

/* -- feedback.html ------------------------------------------------------ */

export const ImageIcon = icon(
  "0 0 24 24",
  <>
    <rect x="3" y="5" width="18" height="14" rx="2" />
    <circle cx="9" cy="10" r="1.6" />
    <path d="m3 16 5-4 4 3 3-2 6 5" />
  </>,
);

/* -- granularity.html --------------------------------------------------- */

/** The stripping guarantee's shield. */
export const ShieldCheckIcon = icon(
  "0 0 24 24",
  <>
    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10Z" />
    <path d="m9 12 2 2 4-4" />
  </>,
);
