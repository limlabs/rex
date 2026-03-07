"use client";

import React, { useState } from 'react';
import { incrementCounter, decrementCounter } from '../app/actions';

export default function ActionCounter() {
  const [count, setCount] = useState(0);
  const [loading, setLoading] = useState(false);

  async function handleIncrement() {
    setLoading(true);
    const next = await incrementCounter(count);
    setCount(next);
    setLoading(false);
  }

  async function handleDecrement() {
    setLoading(true);
    const next = await decrementCounter(count);
    setCount(next);
    setLoading(false);
  }

  return (
    <div>
      <p>Server Action Count: {count}</p>
      <button onClick={handleIncrement} disabled={loading}>+</button>
      <button onClick={handleDecrement} disabled={loading}>-</button>
      {loading && <span>Loading...</span>}
    </div>
  );
}
