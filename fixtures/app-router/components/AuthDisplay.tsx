"use client";

import React from 'react';
import { useAuth } from './AuthProvider';

export default function AuthDisplay() {
  const { user } = useAuth();

  return (
    <div data-testid="auth-display">
      <span>User: {user ?? 'anonymous'}</span>
    </div>
  );
}
