import React from 'react';
import { ThemeProvider } from '../../../components/ThemeProvider';
import { AuthProvider } from '../../../components/AuthProvider';
import ThemeDisplay from '../../../components/ThemeDisplay';
import AuthDisplay from '../../../components/AuthDisplay';

export default function NestedContextPage() {
  return (
    <div>
      <h1>Nested Context Test</h1>
      <ThemeProvider initialTheme="dark">
        <AuthProvider initialUser="rex-user">
          <ThemeDisplay />
          <AuthDisplay />
        </AuthProvider>
      </ThemeProvider>
    </div>
  );
}
