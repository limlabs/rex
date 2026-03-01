import React from 'react';

interface HtmlProps extends React.HTMLAttributes<HTMLHtmlElement> {
  children?: React.ReactNode;
}

interface HeadProps {
  children?: React.ReactNode;
}

export function Html({ children, ...props }: HtmlProps): React.ReactElement {
  return React.createElement('html', props, children);
}

export function Head({ children }: HeadProps): React.ReactElement {
  return React.createElement('head', null, children);
}

export function Main(): React.ReactElement {
  return React.createElement('div', { id: '__rex' });
}

export function NextScript(): null {
  return null;
}

export default function Document(): React.ReactElement {
  return React.createElement(
    Html,
    null,
    React.createElement(Head, null),
    React.createElement(
      'body',
      null,
      React.createElement(Main, null),
      React.createElement(NextScript, null)
    )
  );
}
