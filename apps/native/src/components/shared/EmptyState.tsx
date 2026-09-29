import { LucideIcon } from 'lucide-react';

import {
  Empty,
  EmptyMedia,
  EmptyDescription,
  EmptyHeader,
  EmptyTitle,
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
        <EmptyMedia variant='icon'>
          <Icon />
        </EmptyMedia>
        <EmptyTitle>{title}</EmptyTitle>
        <EmptyDescription>{description}</EmptyDescription>
      </EmptyHeader>
    </Empty>
  );
};
