import React from "react";

// Custom sign-in page.
// Configured via auth.pages.signIn in rex.config.json.

export default function Login() {
  return (
    <div style={{ maxWidth: 400, margin: "80px auto", fontFamily: "system-ui", textAlign: "center" }}>
      <h1>Sign In</h1>
      <p style={{ color: "#666", marginBottom: 32 }}>
        Choose a provider to continue
      </p>

      <a
        href="/_rex/auth/signin?provider=github"
        style={{
          display: "inline-block",
          padding: "12px 24px",
          background: "#24292e",
          color: "white",
          textDecoration: "none",
          borderRadius: 6,
          fontSize: 16,
        }}
      >
        Sign in with GitHub
      </a>

      <p style={{ marginTop: 32 }}>
        <a href="/" style={{ color: "#666" }}>Back to home</a>
      </p>
    </div>
  );
}
