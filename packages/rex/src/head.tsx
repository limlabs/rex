import type { ReactNode } from 'react';

interface HeadProps {
  children?: ReactNode;
}

/**
 * rex/head - Inject elements into the document <head>.
 *
 * On the server, collects <title>, <meta>, etc. for SSR injection.
 * On the client, renders nothing (head is already in the HTML).
 */
export default function Head(_props: HeadProps): null {
  return null;
}
