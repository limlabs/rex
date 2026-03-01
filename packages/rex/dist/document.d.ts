import React from 'react';
interface HtmlProps extends React.HTMLAttributes<HTMLHtmlElement> {
    children?: React.ReactNode;
}
interface HeadProps {
    children?: React.ReactNode;
}
export declare function Html({ children, ...props }: HtmlProps): React.ReactElement;
export declare function Head({ children }: HeadProps): React.ReactElement;
export declare function Main(): React.ReactElement;
export declare function NextScript(): null;
export default function Document(): React.ReactElement;
export {};
//# sourceMappingURL=document.d.ts.map