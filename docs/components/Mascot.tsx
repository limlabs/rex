"use client";

import React, { useState, useEffect } from "react";

const IDLE = [
  "      ▄████▄",
  "      █ ◦ █▀█▄",
  "▄▄▄▄▄▄█████▀▀",
  "  ▀▀▀▀██████",
  "      █▀ █▀",
].join("\n");

const FLICK = [
  "      ▄████▄",
  "▄     █ ◦ █▀█▄",
  " ▀▄▄▄▄█████▀▀",
  "  ▀▀▀▀██████",
  "      █▀ █▀",
].join("\n");

export default function Mascot() {
  const [frame, setFrame] = useState(IDLE);

  useEffect(() => {
    const t1 = setTimeout(() => setFrame(FLICK), 400);
    const t2 = setTimeout(() => setFrame(IDLE), 700);
    return () => {
      clearTimeout(t1);
      clearTimeout(t2);
    };
  }, []);

  return (
    <pre
      className="shrink-0 flex items-center justify-center !rounded-lg !px-8 !py-0"
      style={{
        color: "rgb(46, 204, 113)",
        lineHeight: 1,
        transform: "scaleY(1.4)",
      }}
      aria-hidden="true"
    >
      {frame}
    </pre>
  );
}
