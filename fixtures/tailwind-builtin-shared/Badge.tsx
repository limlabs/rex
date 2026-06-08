import React from 'react';

// This component lives OUTSIDE the tailwind-builtin app's project root. It is
// reachable only through an out-of-root `@source "../../tailwind-builtin-shared"`
// directive in the app's globals.css. The `bg-rose-700` utility below is used
// nowhere else, so it appears in the compiled CSS only if Rex honors that
// out-of-root @source directive (regression guard for limlabs/rex#246).
export default function Badge() {
  return <span className="bg-rose-700 text-white rounded px-2 py-1">Shared</span>;
}
