import React from 'react';
import { ThemeProvider } from '../components/ThemeProvider';
import ThemeDisplay from '../components/ThemeDisplay';

export default function ContextPage() {
  return (
    <div>
      <h1>Context Test</h1>
      <ThemeProvider initialTheme="dark">
        <ThemeDisplay />
      </ThemeProvider>
    </div>
  );
}
