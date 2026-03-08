import React from 'react';
import { headers } from 'rex/actions';

// Server component that uses dynamic functions (headers)
// This forces the route to be server-rendered (not static)
export default function Profile() {
  const h = headers();
  const ua = h['user-agent'] || 'unknown';

  return (
    <div>
      <h1>Profile</h1>
      <p>User Agent: {ua}</p>
    </div>
  );
}
