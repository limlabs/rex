import React from 'react';

// Async server component — tests the RSC async resolution pipeline
export default async function DataPage() {
  const data = await Promise.resolve({ message: "Hello from async server component" });
  return (
    <div>
      <h1>Data Page</h1>
      <p>{data.message}</p>
    </div>
  );
}
