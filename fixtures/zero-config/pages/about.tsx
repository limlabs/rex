import React from "react";

export async function getServerSideProps() {
  console.log("hello limothy");
  return { props: {} };
}

export default function About() {
  return (
    <div>
      <h1>About</h1>
      <p>This page works without installing any npm packages.</p>
      <a href="/">Back to home</a>
    </div>
  );
}
