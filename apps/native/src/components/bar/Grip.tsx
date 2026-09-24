import { GripVerticalIcon } from '@marvis/ui';

/** The row's drag affordance — zero-width until the capsule/card is
 *  hovered (`group-hover/bar`), then slides in as a grab handle. */
export const Grip = () => (
  <span
    className='-mx-0.75 grid w-4 max-w-0 flex-none cursor-grab place-items-center overflow-hidden text-muted-foreground opacity-0 transition-[max-width_var(--motion-base)_var(--ease),opacity_var(--motion-fast)_var(--ease),margin-inline_var(--motion-base)_var(--ease)] group-hover/bar:mx-0 group-hover/bar:max-w-4 group-hover/bar:opacity-100 active:cursor-grabbing motion-reduce:transition-none'
    data-tauri-drag-region='deep'
    title='Drag'
    aria-hidden='true'>
    <GripVerticalIcon className='size-3.75' />
  </span>
);
