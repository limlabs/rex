import React from 'react';

// Nested layout for dashboard section
export default function DashboardLayout({ children }: { children: React.ReactNode }) {
  return (
    <div>
      <div style={{ padding: '8px', background: '#f0f0f0' }}>
        <strong>Dashboard</strong>
        {' — '}
        <a href="/dashboard">Home</a>
        {' | '}
        <a href="/dashboard/settings">Settings</a>
      </div>
      <div>{children}</div>
    </div>
  );
}
