import React from "react";

interface Props {
  utf8: string;
  base64: string;
  hex: string;
  roundtrip: string;
  isBuffer: boolean;
  concat: string;
}

export default function BufferTest({
  utf8,
  base64,
  hex,
  roundtrip,
  isBuffer,
  concat,
}: Props) {
  return (
    <div>
      <h1>Buffer Polyfill Test</h1>
      <p data-testid="utf8">{utf8}</p>
      <p data-testid="base64">{base64}</p>
      <p data-testid="hex">{hex}</p>
      <p data-testid="roundtrip">{roundtrip}</p>
      <p data-testid="is-buffer">{String(isBuffer)}</p>
      <p data-testid="concat">{concat}</p>
    </div>
  );
}

export function getServerSideProps() {
  const buf = Buffer.from("hello world");
  const b64Buf = Buffer.from("SGVsbG8gUmV4IQ==", "base64");
  const a = Buffer.from("foo");
  const b = Buffer.from("bar");

  return {
    props: {
      utf8: buf.toString("utf8"),
      base64: buf.toString("base64"),
      hex: buf.toString("hex"),
      roundtrip: b64Buf.toString("utf8"),
      isBuffer: Buffer.isBuffer(buf),
      concat: Buffer.concat([a, b]).toString("utf8"),
    },
  };
}
