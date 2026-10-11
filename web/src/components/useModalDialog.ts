import { useEffect, useRef, type MouseEvent, type PointerEvent, type SyntheticEvent } from "react";

/** Native dialog owns focus and inertness; this hook handles lifecycle and backdrop dismissal. */
export function useModalDialog(onDismiss: () => void) {
  const ref = useRef<HTMLDialogElement>(null);
  const closingInternally = useRef(false);
  const outsidePress = useRef(false);
  useEffect(() => {
    const dialog = ref.current!;
    closingInternally.current = false;
    dialog.showModal();
    return () => {
      closingInternally.current = true;
      if (dialog.open) dialog.close();
    };
  }, []);

  function outside(event: MouseEvent<HTMLDialogElement>) {
    if (event.target !== event.currentTarget) return false;
    const bounds = event.currentTarget.getBoundingClientRect();
    return (
      event.clientX < bounds.left ||
      event.clientX > bounds.right ||
      event.clientY < bounds.top ||
      event.clientY > bounds.bottom
    );
  }

  return {
    ref,
    onPointerDown(event: PointerEvent<HTMLDialogElement>) {
      outsidePress.current = event.button === 0 && outside(event);
    },
    onClick(event: MouseEvent<HTMLDialogElement>) {
      // Both ends must be outside: dragging content out must not dismiss it.
      if (outsidePress.current && event.button === 0 && outside(event)) event.currentTarget.close();
      outsidePress.current = false;
    },
    onClose(event: SyntheticEvent<HTMLDialogElement>) {
      // Strict Mode closes during cleanup, then reopens. That close event can
      // arrive after reopen; it must not dismiss the newly opened dialog.
      if (closingInternally.current || event.currentTarget.open) return;
      onDismiss();
    },
  };
}
