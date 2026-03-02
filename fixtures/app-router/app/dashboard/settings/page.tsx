import React from 'react';
import Counter from '../../../components/Counter';

// Server component that imports a "use client" component
export default function Settings() {
  return (
    <div>
      <h1>Settings</h1>
      <p>Adjust your preferences below.</p>
      <Counter />
    </div>
  );
}
