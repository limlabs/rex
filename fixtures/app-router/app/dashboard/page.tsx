import React from 'react';
import Link from 'rex/link';

// Server component — dashboard home
export default function Dashboard() {
  return (
    <div>
      <h1>Dashboard</h1>
      <p>Welcome to your dashboard.</p>
      <Link href="/dashboard/settings">Settings</Link>
    </div>
  );
}
