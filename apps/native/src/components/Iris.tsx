/** The iris — Marvis' presence mark (DESIGN.md §6). The ::before pupil
   breathes; `data-expanded` on the pill (`group/bar`) folds it behind
   the back arrow during the icon-row⇄input-row swap. */
export const Iris = () => (
  <span
    aria-hidden
    className='relative grid size-4.5 flex-none place-items-center transition-[scale_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease)] group-data-expanded/bar:scale-30 group-data-expanded/bar:opacity-0 motion-reduce:transition-none before:size-2.75 before:animate-iris-breath before:rounded-full before:bg-fg-2 before:content-[""] after:absolute after:size-[4.5px] after:rounded-full after:bg-surface after:content-[""] motion-reduce:before:animate-none'
  />
);
