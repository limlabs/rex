import React from 'react';

interface Props {
  message: string;
  timestamp: string;
}

export default function Home({ message, timestamp }: Props) {
  return (
    <div className="max-w-2xl mx-auto p-8">
      <h1 className="text-4xl font-bold text-gray-900 mb-4">Tailwind Builtin</h1>
      <p className="text-lg text-gray-600 mb-2">{message}</p>
      <p className="text-sm text-gray-400">Rendered at: {timestamp}</p>
      <div className="mt-8 grid grid-cols-2 gap-4">
        <div className="bg-white rounded-lg shadow p-6">
          <h2 className="text-xl font-semibold mb-2">Zero Install</h2>
          <p className="text-gray-500">No npm install needed for Tailwind</p>
        </div>
        <div className="bg-white rounded-lg shadow p-6">
          <h2 className="text-xl font-semibold mb-2">Built-in</h2>
          <p className="text-gray-500">Tailwind compiled via embedded V8</p>
        </div>
      </div>
    </div>
  );
}

export async function getServerSideProps() {
  return {
    props: {
      message: 'Hello from Rex with built-in Tailwind!',
      timestamp: new Date().toISOString(),
    },
  };
}
