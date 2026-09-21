import { Fragment, type ReactNode } from 'react';

/* Renders lines inside a white-space: pre/pre-wrap host where newlines and a
 * fixed leading indent are content. JSX collapses whitespace, so each is
 * emitted explicitly: '\n' separates lines, `indent` prefixes non-empty ones
 * (null entries render as truly empty lines). Used by the hero editor and the
 * ~/.marvis file tree — both faithful to the reference markup's indentation. */
export const PreLines = ({
  lines,
  indent,
}: {
  lines: ReactNode[];
  indent: string;
}) => (
  <>
    {lines.map((line, i) => (
      <Fragment key={i}>
        {i > 0 && '\n'}
        {i > 0 && line !== null && indent}
        {line}
      </Fragment>
    ))}
  </>
);
