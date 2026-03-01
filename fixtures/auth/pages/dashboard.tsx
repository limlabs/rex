import React from "react";

// Protected page — requires authentication.
// Redirects to /login if the user is not signed in.

export async function getServerSideProps(context: any) {
  if (!context.session) {
    return {
      redirect: {
        destination: "/login",
        permanent: false,
      },
    };
  }

  return {
    props: {
      user: context.session.user,
      provider: context.session.provider,
    },
  };
}

export default function Dashboard({
  user,
  provider,
}: {
  user: { id: string; name?: string; email?: string; image?: string };
  provider: string;
}) {
  return (
    <div style={{ maxWidth: 600, margin: "40px auto", fontFamily: "system-ui" }}>
      <h1>Dashboard</h1>
      <p>This page is only visible to authenticated users.</p>

      <div
        style={{
          background: "#f5f5f5",
          padding: 20,
          borderRadius: 8,
          marginTop: 20,
        }}
      >
        <h2>Your Profile</h2>
        {user.image && (
          <img
            src={user.image}
            alt="avatar"
            style={{ width: 64, height: 64, borderRadius: "50%" }}
          />
        )}
        <table style={{ marginTop: 12 }}>
          <tbody>
            <tr>
              <td style={{ fontWeight: "bold", paddingRight: 16 }}>ID</td>
              <td>{user.id}</td>
            </tr>
            {user.name && (
              <tr>
                <td style={{ fontWeight: "bold", paddingRight: 16 }}>Name</td>
                <td>{user.name}</td>
              </tr>
            )}
            {user.email && (
              <tr>
                <td style={{ fontWeight: "bold", paddingRight: 16 }}>Email</td>
                <td>{user.email}</td>
              </tr>
            )}
            <tr>
              <td style={{ fontWeight: "bold", paddingRight: 16 }}>Provider</td>
              <td>{provider}</td>
            </tr>
          </tbody>
        </table>
      </div>

      <div style={{ marginTop: 24 }}>
        <a href="/">Home</a>
        {" | "}
        <form
          action="/_rex/auth/signout"
          method="post"
          style={{ display: "inline" }}
        >
          <button type="submit" style={{ cursor: "pointer" }}>
            Sign Out
          </button>
        </form>
      </div>
    </div>
  );
}
