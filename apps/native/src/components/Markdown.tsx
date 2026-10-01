import { openUrl } from '@tauri-apps/plugin-opener';
import ReactMarkdown from 'react-markdown';
import remarkGfm from 'remark-gfm';

/** Lazy boundary for the react-markdown + remark-gfm stack (~200 kB) —
 *  `ChatSection` dynamic-imports this so it stays out of the entry
 *  chunk every window parses. */
export const Markdown = ({ children }: { children: string }) => (
  <ReactMarkdown
    remarkPlugins={[remarkGfm]}
    disallowedElements={['img']}
    components={{
      a: ({ href, children }) => (
        <a
          href={href}
          onClick={(e) => {
            e.preventDefault();
            if (href) void openUrl(href);
          }}>
          {children}
        </a>
      ),
    }}>
    {children}
  </ReactMarkdown>
);
