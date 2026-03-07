import React from 'react';
import Link from 'rex/link';

// Nested layout for dashboard section
export default function DashboardLayout({ children }: { children: React.ReactNode }) {
  return (
    <div>
      <div style={{ padding: '8px', background: '#f0f0f0' }}>
        <strong>Dashboard</strong>
        {' — '}
        <Link href="/dashboard">Home</Link>
        {' | '}
        <Link href="/dashboard/settings">Settings</Link>
      </div>
      <div>{children}</div>
    </div>
  );
}
