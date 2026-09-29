import {
  Empty,
  EmptyMedia,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
  type LucideIcon,
} from '@marvis/ui';

type Props = {
  icon: LucideIcon;
  title: string;
  description: string;
};

export const EmptyState = ({ icon: Icon, title, description }: Props) => {
  return (
    <Empty>
      <EmptyHeader>
        <EmptyMedia
          variant='icon'
          className='bg-accent/20'>
          <Icon className='text-accent/40 size-5' />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
};
