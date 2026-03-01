import React from "react";

// Public home page — visible to everyone.
// Shows different content based on whether the user is logged in.

export async function getServerSideProps(context: any) {
  return {
    props: {
      session: context.session || null,
    },
  };
}

export default function Home({ session }: { session: any }) {
  return (
    <div style={{ maxWidth: 600, margin: "40px auto", fontFamily: "system-ui" }}>
      <h1>Rex Auth Example</h1>
      <p>This app demonstrates Rex built-in authentication with GitHub OAuth.</p>

      {session ? (
        <div>
          <p>
            Welcome, <strong>{session.user.name || session.user.email || session.user.id}</strong>!
          </p>
          <p>
            <a href="/dashboard">Go to Dashboard</a>
          </p>
          <form action="/_rex/auth/signout" method="post">
            <button type="submit">Sign Out</button>
          </form>
        </div>
      ) : (
        <div>
          <p>You are not signed in.</p>
          <a href="/_rex/auth/signin?provider=github">Sign in with GitHub</a>
        </div>
      )}

      <hr style={{ margin: "24px 0" }} />
      <h2>How it works</h2>
      <ul>
        <li>
          <code>getServerSideProps</code> reads <code>context.session</code> to
          check if the user is authenticated
        </li>
        <li>
          <code>/_rex/auth/signin?provider=github</code> redirects to GitHub
          for OAuth sign-in
        </li>
        <li>
          <code>/_rex/auth/signout</code> clears the session cookie
        </li>
        <li>
          The <code>/dashboard</code> page is protected — it redirects
          unauthenticated users to <code>/login</code>
        </li>
      </ul>
    </div>
  );
}
