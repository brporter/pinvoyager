/**
 * The PinVoyager component library.
 *
 * Every component here implements a file in `design/components/` and carries
 * the same name as it, so a design file and its implementation are findable
 * from each other:
 *
 * | design file                          | components                                  |
 * |--------------------------------------|---------------------------------------------|
 * | `buttons.html`                       | `Button`, `Spinner`                         |
 * | `provider-buttons.html`              | `ProviderButton`                            |
 * | `forms.html`                         | `TextField`, `TextArea`, `Select`, `Checkbox`, `Switch` |
 * | `navigation.html`                    | `TopNav`, `TabBar`, `Avatar`                |
 * | `photo-primitives.html`              | `Photo`, `PhotoGrid`, `Filmstrip`           |
 * | `map-primitives.html`                | `Pin`, `Cluster`, `MapArea`, `Attribution`  |
 * | `upload.html`                        | `Dropzone`, `UploadRow`                     |
 * | `granularity.html`                   | `GranularitySelector`                       |
 * | `feedback.html`                      | `Badge`, `Toast`, `EmptyState`, `Skeleton`  |
 *
 * None of them define their own styles. They apply the class names in
 * `design/components.css`, which the application imports directly from the
 * design directory — so editing the design system changes the running
 * application rather than a copy of it. See `src/styles/index.css`.
 */

export { Button, Spinner } from "./Button";
export type { ButtonProps, ButtonSize, ButtonVariant } from "./Button";

export { PROVIDERS, ProviderButton } from "./ProviderButton";
export type { Provider, ProviderButtonProps } from "./ProviderButton";

export { TextField } from "./TextField";
export type { TextFieldProps } from "./TextField";
export { TextArea } from "./TextArea";
export type { TextAreaProps } from "./TextArea";
export { Select } from "./Select";
export type { SelectProps } from "./Select";
export { Checkbox } from "./Checkbox";
export type { CheckboxProps } from "./Checkbox";
export { Switch } from "./Switch";
export type { SwitchProps } from "./Switch";
export type { FieldChrome } from "./Field";

export { TopNav } from "./TopNav";
export type { NavItem, NavUser, TopNavProps } from "./TopNav";
export { DEFAULT_TABS, TabBar } from "./TabBar";
export type { TabBarProps, TabItem } from "./TabBar";
export { Avatar, initialsOf } from "./Avatar";
export type { AvatarProps } from "./Avatar";

export { Photo } from "./Photo";
export type { PhotoProps, PhotoRatio } from "./Photo";
export { PhotoGrid } from "./PhotoGrid";
export type { PhotoGridProps } from "./PhotoGrid";
export { Filmstrip } from "./Filmstrip";
export type { FilmstripProps } from "./Filmstrip";

export { Attribution, Cluster, MapArea, Pin } from "./MapPrimitives";
export type {
  AttributionProps,
  ClusterProps,
  ClusterSize,
  MapAreaProps,
  MapCoord,
  PinProps,
} from "./MapPrimitives";

export { Dropzone } from "./Dropzone";
export type { DropzoneProps, DropzoneState } from "./Dropzone";
export { UploadRow } from "./UploadRow";
export type { UploadRowProps, UploadStatus } from "./UploadRow";

export {
  GRANULARITIES,
  GRANULARITY_LABEL,
  GranularitySelector,
} from "./GranularitySelector";
export type { GranularitySelectorProps } from "./GranularitySelector";

export { Badge } from "./Badge";
export type { BadgeProps, BadgeTone } from "./Badge";
export { Toast } from "./Toast";
export type { ToastProps, ToastTone } from "./Toast";
export { EmptyState } from "./EmptyState";
export type { EmptyStateProps } from "./EmptyState";
export { Skeleton } from "./Skeleton";
export type { SkeletonProps } from "./Skeleton";

export * from "./icons";
export { cx } from "./cx";
