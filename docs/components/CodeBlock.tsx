import React from "react";
import Mermaid from "./Mermaid";
import hljs from "highlight.js/lib/core";
import bash from "highlight.js/lib/languages/bash";
import css from "highlight.js/lib/languages/css";
import dockerfile from "highlight.js/lib/languages/dockerfile";
import javascript from "highlight.js/lib/languages/javascript";
import json from "highlight.js/lib/languages/json";
import rust from "highlight.js/lib/languages/rust";
import typescript from "highlight.js/lib/languages/typescript";
import xml from "highlight.js/lib/languages/xml";
import yaml from "highlight.js/lib/languages/yaml";

hljs.registerLanguage("bash", bash);
hljs.registerLanguage("sh", bash);
hljs.registerLanguage("css", css);
hljs.registerLanguage("dockerfile", dockerfile);
hljs.registerLanguage("javascript", javascript);
hljs.registerLanguage("js", javascript);
hljs.registerLanguage("json", json);
hljs.registerLanguage("rust", rust);
hljs.registerLanguage("typescript", typescript);
hljs.registerLanguage("ts", typescript);
hljs.registerLanguage("tsx", typescript);
hljs.registerLanguage("jsx", javascript);
hljs.registerLanguage("xml", xml);
hljs.registerLanguage("html", xml);
hljs.registerLanguage("yaml", yaml);
hljs.registerLanguage("toml", yaml);
hljs.registerLanguage("mdx", javascript);

interface CodeBlockProps {
  children?: React.ReactNode;
}

export default function CodeBlock({ children }: CodeBlockProps) {
  // MDX renders: <pre><code className="language-x">...</code></pre>
  // Extract the language and raw code from the <code> child
  const child = React.Children.only(children) as React.ReactElement<{
    className?: string;
    children?: string;
  }>;
  const className = child?.props?.className || "";
  const lang = className.replace("language-", "");
  const code = child?.props?.children || "";

  // Render mermaid diagrams
  if (lang === "mermaid") {
    return <Mermaid chart={code.trim()} />;
  }

  // Highlight if we recognize the language, otherwise render plain
  if (lang && hljs.getLanguage(lang)) {
    const highlighted = hljs.highlight(code, { language: lang }).value;
    return (
      <pre>
        <code
          className={`hljs ${className}`}
          dangerouslySetInnerHTML={{ __html: highlighted }}
        />
      </pre>
    );
  }

  return (
    <pre>
      <code className={className}>{code}</code>
    </pre>
  );
}
