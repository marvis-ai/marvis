/**
 * Caret geometry for the composer textarea — used to anchor the
 * preset palette's left edge to the insertion point.
 */

/** The caret's x in the field's window-viewport px (Rust adds the
 *  window's outer position for the screen anchor). Canvas-measured
 *  with the element's own font — no mirror DOM. A multi-line value
 *  measures only the caret's line (the pill's wrapped second line
 *  starts back at the left padding), and horizontal scroll is
 *  subtracted so a scrolled caret reports its VISIBLE position. */
let caretCtx: CanvasRenderingContext2D | null | undefined;

export const caretViewportX = (el: HTMLTextAreaElement): number => {
  const cs = getComputedStyle(el);
  caretCtx ??= document.createElement('canvas').getContext('2d');
  let w = 0;
  if (caretCtx) {
    caretCtx.font = `${cs.fontWeight} ${cs.fontSize} ${cs.fontFamily}`;
    const before = el.value.slice(0, el.selectionStart ?? 0);
    const line = before.slice(before.lastIndexOf('\n') + 1);
    w = caretCtx.measureText(line).width;
  }
  return (
    el.getBoundingClientRect().x +
    parseFloat(cs.paddingLeft) +
    w -
    el.scrollLeft
  );
};
