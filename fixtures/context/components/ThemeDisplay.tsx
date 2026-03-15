"use client";

import React from 'react';
import { useTheme } from './ThemeProvider';

export default function ThemeDisplay() {
  const { theme } = useTheme();

  return (
    <div data-testid="theme-display">
      <span>Current theme: {theme}</span>
    </div>
  );
}
